#!/usr/bin/env python3
"""探测两个开工前必须知道的线格式事实。

1. `assistant-stream` 帧里，增量文字到底落在哪个字段。
   （`transcript_controller.dart` 现在只读了 `start` / `end`。）
2. 文件 / 图片上传的线格式。
   参考客户端锁在 harness 0.1.2-alpha.1，本机部署是 qianxun 0.7.0，
   两者的载荷形状对不上（见 README §2.2.1）。**不要凭记忆写。**

用法::

    python scripts\\probe_stream.py --token <64位hex> --session <sessionId> [--seconds 40]
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import socket
import sys
import time
import urllib.request

MUX_PATH = "/api/remote.mux"


def call(host, port, token, method, args, clip=600):
    body = json.dumps(
        {"type": "client-request", "rpcId": "p", "method": method, "payload": {"args": args}}
    ).encode()
    req = urllib.request.Request(
        f"http://{host}:{port}/api/{method}",
        data=body,
        method="POST",
        headers={"Cookie": f"qx_token={token}", "Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            return r.read().decode("utf-8", "replace")[:clip]
    except Exception as e:  # noqa: BLE001
        return f"{type(e).__name__}: {e}"


def ws_connect(host, port, token):
    """裸 WS 握手。返回 (socket, 收缓冲)。"""
    key = base64.b64encode(os.urandom(16)).decode()
    s = socket.create_connection((host, port), timeout=15)
    s.sendall(
        (
            f"GET {MUX_PATH} HTTP/1.1\r\n"
            f"Host: {host}:{port}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n"
            f"Cookie: qx_token={token}\r\n\r\n"
        ).encode()
    )
    head = b""
    while b"\r\n\r\n" not in head:
        chunk = s.recv(4096)
        if not chunk:
            break
        head += chunk
    if b"101" not in head:
        raise RuntimeError(f"handshake failed: {head[:200]!r}")
    rest = head.split(b"\r\n\r\n", 1)[1]
    return s, rest


def ws_send(s, obj):
    """客户端→服务端必须是**掩码**帧。"""
    payload = json.dumps(obj).encode()
    mask = os.urandom(4)
    n = len(payload)
    if n < 126:
        hdr = bytes([0x81, 0x80 | n])
    elif n < 65536:
        hdr = bytes([0x81, 0x80 | 126]) + n.to_bytes(2, "big")
    else:
        hdr = bytes([0x81, 0x80 | 127]) + n.to_bytes(8, "big")
    masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
    s.sendall(hdr + mask + masked)


def ws_frames(buf, s, want, seconds):
    """攒够 want 帧或超时。返回解析出的 (opcode, payload) 列表。"""
    out = []
    buf = bytearray(buf)
    end = time.time() + seconds
    while time.time() < end and len(out) < want:
        s.settimeout(max(0.2, end - time.time()))
        try:
            chunk = s.recv(65536)
            if not chunk:
                break
            buf.extend(chunk)
        except socket.timeout:
            break
        while True:
            if len(buf) < 2:
                break
            b0, b1 = buf[0], buf[1]
            op = b0 & 0x0F
            ln = b1 & 0x7F
            off = 2
            if ln == 126:
                if len(buf) < 4:
                    break
                ln = int.from_bytes(buf[2:4], "big")
                off = 4
            elif ln == 127:
                if len(buf) < 10:
                    break
                ln = int.from_bytes(buf[2:10], "big")
                off = 10
            if b1 & 0x80:
                if len(buf) < off + 4:
                    break
                mask = bytes(buf[off : off + 4])
                off += 4
            if len(buf) < off + ln:
                break
            data = bytes(buf[off : off + ln])
            if b1 & 0x80:
                data = bytes(b ^ mask[i % 4] for i, b in enumerate(data))
            del buf[: off + ln]
            if op == 0x1:
                out.append(data.decode("utf-8", "replace"))
    return out, bytes(buf)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=23090)
    ap.add_argument("--token", required=True)
    ap.add_argument("--session", help="要观察的 sessionId；不给就自动取第一个")
    ap.add_argument("--seconds", type=int, default=40)
    args = ap.parse_args()

    sid = args.session
    if not sid:
        out = call(args.host, args.port, args.token, "session/list", {"_request": {}}, clip=100000)
        try:
            items = json.loads(out)["result"]["value"]["items"]
            sid = items[0]["sessionId"]
        except Exception:  # noqa: BLE001
            print("拿不到 sessionId:", out[:300])
            return 1
    print(f"观察会话：{sid}\n")

    s, rest = ws_connect(args.host, args.port, args.token)
    ws_send(s, {"type": "open", "streamId": "1", "endpoint": "$events", "payload": {"args": {}}})
    ws_send(
        s,
        {
            "type": "open",
            "streamId": "2",
            "endpoint": "session/follow",
            "payload": {
                "args": {
                    "address": {"kind": "session", "sessionId": sid},
                    "assistantStream": True,
                }
            },
        },
    )
    print(f"已订阅 $events + session/follow，监听 {args.seconds}s。")
    print("提示：现在去电脑上让这个会话跑起来（发一条消息），效果最好。\n")

    frames, _ = ws_frames(rest, s, want=40, seconds=args.seconds)
    kinds: dict[str, int] = {}
    samples: list[str] = []
    for f in frames:
        try:
            o = json.loads(f)
        except Exception:  # noqa: BLE001
            continue
        key = f"{o.get('type')}"
        if key == "item":
            key += ":" + str(o.get("streamId"))
        kinds[key] = kinds.get(key, 0) + 1
        if key.startswith("item:2") and len(samples) < 6:
            samples.append(f[:700])

    print("=== 收到的帧类型统计 ===")
    for k, v in sorted(kinds.items(), key=lambda x: -x[1]):
        print(f"  {k:<28} {v}")
    print("\n=== session/follow 的样本（找增量文字字段）===")
    for sline in samples:
        print("  " + sline)
        print()
    s.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
