#!/usr/bin/env python3
"""探测「上传 → 引用 → 发送」这条链路的每一跳。

已实测确认的事实（见本文末尾的发现表）：
- `POST /api/session/uploadFileBinary?sessionId=…&name=…`
  `Content-Type: application/octet-stream`，**裸二进制 body**
  ⇒ **没有** `{"args":{…}}` 信封，也不走 client-request 信封
- 返回 `{ok, value:{receiptId, file:{attachmentId, name, bytes}}}`

本脚本把剩下的两跳（receiptId 怎么进 `session/prompt` 的 content）也测出来。
"""

from __future__ import annotations

import argparse
import base64
import json
import urllib.request

# 1x1 png
PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="
)


def call_rpc(host, port, token, method, args, wrap="request"):
    payload = {"args": args if wrap is None else {wrap: args}}
    body = json.dumps(
        {"type": "client-request", "rpcId": "probe", "method": method, "payload": payload}
    ).encode()
    req = urllib.request.Request(
        f"http://{host}:{port}/api/{method}",
        data=body,
        method="POST",
        headers={"Cookie": f"qx_token={token}", "Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(req, timeout=25) as r:
            return r.read().decode("utf-8", "replace")
    except Exception as e:  # noqa: BLE001
        t = ""
        try:
            t = e.read().decode("utf-8", "replace")
        except Exception:  # noqa: BLE001
            pass
        return f"HTTP {getattr(e, 'code', '')} {t}"


def upload(host, port, token, session_id, data, name):
    url = f"http://{host}:{port}/api/session/uploadFileBinary?sessionId={session_id}&name={name}"
    req = urllib.request.Request(
        url,
        data=data,
        method="POST",
        headers={"Cookie": f"qx_token={token}", "Content-Type": "application/octet-stream"},
    )
    with urllib.request.urlopen(req, timeout=30) as r:
        return r.read().decode("utf-8", "replace")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=23090)
    ap.add_argument("--token", required=True)
    ap.add_argument("--session", help="不给就自动取第一个")
    ap.add_argument(
        "--send",
        action="store_true",
        help="真的把消息发出去（会污染会话历史）。默认只做 schema 探测，不发。",
    )
    args = ap.parse_args()

    sid = args.session
    if not sid:
        raw = call_rpc(args.host, args.port, args.token, "session/list", {}, wrap="_request")
        sid = json.loads(raw)["result"]["value"]["items"][0]["sessionId"]
    print(f"会话：{sid}\n")

    print("=== 1. 上传（裸二进制 + query sessionId）===")
    up_raw = upload(args.host, args.port, args.token, sid, PNG, "probe.png")
    print(up_raw[:300])
    val = json.loads(up_raw)["value"]
    rid, aid = val["receiptId"], val["file"]["attachmentId"]
    print(f"\n  receiptId = {rid}")
    print(f"  attachmentId = {aid}\n")

    print("=== 2. receiptId 进 content 的候选写法 ===")
    candidates = [
        ("file + receiptId", {"type": "file", "receiptId": rid}),
        ("image + receiptId", {"type": "image", "receiptId": rid}),
        (
            "file + mediaType + name + receiptId",
            {"type": "file", "receiptId": rid, "mediaType": "image/png", "name": "probe.png"},
        ),
    ]
    for label, part in candidates:
        if not args.send:
            print(f"  -- {label} （--send 才真发）")
            print(f"     {json.dumps(part, ensure_ascii=False)}")
            continue
        out = call_rpc(
            args.host,
            args.port,
            args.token,
            "session/prompt",
            {
                "requestId": f"probe-{rid[:8]}-{label.split()[0]}",
                "sessionId": sid,
                "mode": "queue",
                "content": [{"type": "text", "text": "[协议探针] 这条可以忽略"}, part],
            },
        )
        print(f"\n  -- {label}")
        print(f"     {out[:400]}")

    if not args.send:
        print("\n（加 --send 会真的发消息进去，确认哪一版被接受）")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
