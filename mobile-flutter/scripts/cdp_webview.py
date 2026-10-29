"""通过 CDP 直连 Android WebView 来检查页面 —— 比坐标点击可靠得多。

为什么不用 `adb shell input tap`
-------------------------------
Preview Notice 点不动。已排除的可能：
  · 它是模态吞掉了点击       —— 但 tap 的坐标正落在 Continue 上
  · 弹窗后于 custom.js 挂载  —— 已加 6 秒 warmup 扫描 + 2 秒轮询
  · 文案是 i18n 键            —— 扫了 13MB bundle 的字典，没有 preview/notice 键
  · 存在 settings/storages   —— 扫过，没有
剩下的最可能：**弹窗渲染在 iframe 或独立 WebView 层**，
`document.querySelectorAll` 够不到，而坐标点击落在错误的窗口上。

CDP 直连后可以：
  · 直接读 iframe 内的 DOM
  · 用 selector 精确点击，不靠坐标
  · 看 console 与网络失败
  · 抓页面截图

用法：
  python scripts/cdp_webview.py ports        # 列出可连的 webview 调试端口
  python scripts/cdp_webview.py eval "document.title"
  python scripts/cdp_webview.py shot .tmp/webview.png
  python scripts/cdp_webview.py click "text=Continue"  （简化选择器）
"""
from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path

ADB = Path(r"D:\programs\android-sdk\platform-tools\adb.exe")
PKG = "com.qianxun.qianxun_mobile"

# 设备序列号。真机与模拟器常常同时连着，所以要能切：
#   python scripts/cdp_webview.py --serial AEQC8P55MFCYBUFA eval "document.title"
#   或设环境变量 QX_SERIAL=AEQC8P55MFCYBUFA
_DEFAULT_SERIAL = "emulator-5554"
SERIAL = os.environ.get("QX_SERIAL", _DEFAULT_SERIAL)

# ── 真机护栏 ────────────────────────────────────────────────────────
# ⚠️ 用户要求：**本阶段不使用真机**（他在手机上做特殊的事）。
#    这个脚本是整套工具里 adb 调用最频繁的一个，
#    所以护栏放在**解析 --serial 之后、真正发 adb 之前**，
#    任何路径（命令行 / 环境变量 / 默认值）都绕不过去。
_FORBIDDEN = ("AEQC8P55MFCYBUFA",)


def _guard_phone(allow: bool) -> None:
    if SERIAL in _FORBIDDEN and not allow:
        raise SystemExit(
            "⛔ 拒绝操作真机 " + SERIAL + "\n"
            "   用户要求本阶段只使用模拟器（" + _DEFAULT_SERIAL + "）。\n"
            "   确实需要时加 --allow-phone 临时放行。")


def _apply_serial_argv(argv: list[str]) -> list[str]:
    """把 --serial/--emulator 从 argv 里摘出来并写进 SERIAL。"""
    global SERIAL
    out: list[str] = []
    i = 0
    while i < len(argv):
        if argv[i] == "--serial" and i + 1 < len(argv):
            SERIAL = argv[i + 1]
            i += 2
            continue
        if argv[i] == "--emulator":
            SERIAL = _DEFAULT_SERIAL
            i += 1
            continue
        if argv[i] == "--allow-phone":
            i += 1
            continue
        out.append(argv[i])
        i += 1
    return out


def adb(*args: str, timeout: int = 30) -> str:
    proc = subprocess.run(
        [str(ADB), "-s", SERIAL, *args],
        capture_output=True, timeout=timeout,
    )
    return (proc.stdout or b"").decode("utf-8", "ignore")


def pick_socket(want_url: str = "") -> tuple[str, int] | None:
    """挑出**属于本项目**的那个 WebView socket，返回 (socket, 本地端口)。

    ⚠️⚠️ 真机上不止一个 WebView —— 实测同一台 OPPO 上同时有：
        webview_devtools_remote_6413   ← 我们的 DSH 页
        webview_devtools_remote_10976  ← iot-cloud 的 VPN 应用（tauri.localhost）
    按名字取第一个会连到 VPN 应用去，看到的是
    `http://tauri.localhost/`「Tauri + Vue + TS」，跟本项目毫无关系，
    而且**看起来一切正常**（能 eval、能截图），只是内容完全不对。
    这种错连比连不上更难发现。

    所以这里逐个 forward 并**按页面 URL 判定**，命中不了就报清楚看到了什么。
    """
    socks = webview_sockets()
    if not socks:
        return None
    seen: list[str] = []
    for s in socks:
        port = forward(s)
        if port is None:
            continue
        try:
            listing = pages(port)
        except Exception:
            continue
        for p in listing:
            url = p.get("url", "")
            if want_url and want_url in url:
                return s, port
            seen.append(f"{s} → {url[:60]}")
    print("没找到属于本项目的 WebView。逐个 forward 看到的是：")
    for line in seen:
        print("   ", line)
    print(f"（要找的 URL 片段：{want_url!r}）")
    return None


def webview_sockets() -> list[str]:
    """从 /proc/net/unix 里找 webview 的 devtools socket。

    ⚠️ `/proc/net/unix` 里的名字**带前导 @**（`@webview_devtools_remote_3396`），
    而 `adb forward` 传的 `localabstract:` **不要那个 @**。
    带上就会 forward 失败（且失败得很安静：forward 列表里看着有，
    但连 127.0.0.1:9222 直接拒绝）。
    """
    out = adb("shell", "cat", "/proc/net/unix")
    found = []
    for line in out.splitlines():
        if "webview_devtools_remote" in line:
            m = re.search(r"(@?webview_devtools_remote_\d+)", line)
            if m:
                found.append(m.group(1).lstrip("@"))
    return found


def forward(sock: str) -> int | None:
    """把 devtools socket 转发到本地端口，返回本地端口。"""
    local = 9222
    adb("forward", f"tcp:{local}", f"localabstract:{sock}")
    # 验证
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{local}/json/list", timeout=5) as r:
            return local if json.loads(r.read().decode()) else None
    except (urllib.error.URLError, OSError):
        return None


def pages(port: int) -> list[dict]:
    with urllib.request.urlopen(f"http://127.0.0.1:{port}/json/list", timeout=8) as r:
        return json.loads(r.read().decode())


def ws_url(port: int, page: dict) -> str:
    # Android WebView 的 debugger URL 不是标准 ws://，要用 webSocketDebuggerUrl
    return page.get("webSocketDebuggerUrl", "")


def main() -> int:
    argv0 = sys.argv[1:]
    _guard_phone("--allow-phone" in argv0)
    sys.argv = _apply_serial_argv(argv0)
    _guard_phone(False)          # 环境变量也可能把 SERIAL 指向真机
    print(f"设备 {SERIAL}")
    if len(sys.argv) < 1:
        print(__doc__)
        return 1
    cmd = sys.argv[0]

    socks = webview_sockets()
    if not socks:
        print("没找到 webview 调试 socket。")
        print("→ WebView 必须开启调试。检查 android:debuggable 或 WebView.setWebContentsDebuggingEnabled。")
        return 1
    print(f"webview sockets: {socks}")

    picked = pick_socket(os.environ.get("QX_URL_MATCH", "192.168."))
    if picked is None:
        return 1
    _sock, port = picked
    print(f"已连上本地端口 {port}")

    listing = pages(port)
    print(f"页面 {len(listing)} 个:")
    for p in listing:
        print(f"  {p.get('type')}  {p.get('title')!r}  {p.get('url','')[:70]}")
        if p.get('type') == 'iframe':
            print(f"     (iframe)")

    target = next((p for p in listing if p.get('type') == 'page'), None)
    if target is None:
        print("没有 page 类型的目标（只有 iframe？那要在 iframe 里执行）")
        return 1

    ws = ws_url(port, target)
    if not ws:
        print("拿不到 webSocketDebuggerUrl")
        return 1

    if cmd == "ports":
        return 0
    if cmd == "shot":
        # 用 Page.captureScreenshot
        return capture(ws, sys.argv[1] if len(sys.argv) > 1 else ".tmp/webview.png")
    if cmd == "eval":
        # `eval --file <路径>`：表达式从文件读。
        # 为什么必须有它：PowerShell 会吞引号、反引号和 $var，
        # 而要 eval 的 JS 往往正好是带模板字符串的（踩过一次，注入的 CSS 整段丢失）。
        if len(sys.argv) >= 3 and sys.argv[1] == "--file":
            path = Path(sys.argv[2])
            if not path.is_file():
                print(f"找不到 {path}")
                return 1
            return evaluate(ws, path.read_text(encoding="utf-8"))
        expr = sys.argv[1] if len(sys.argv) > 1 else "document.title"
        return evaluate(ws, expr)

    print(__doc__)
    return 1


# ---- CDP over WebSocket（不引第三方依赖）----
import base64
import hashlib
import os
import socket
import struct


class WS:
    def __init__(self, url: str):
        # ws://host:port/devtools/page/XXX
        m = re.match(r"ws://([^/:]+):(\d+)(/.*)", url)
        if not m:
            # Android 常见形态：ws://127.0.0.1:PORT/devtools/page/ID 或裸 ID
            raise ValueError(f"无法解析 ws url: {url}")
        host, port, path = m.group(1), int(m.group(2)), m.group(3)
        # ⚠️ 20s 在真机上不够：手机走 VPN 隧道到桌面，实测 RTT 230–600ms
        #    （ping 192.168.20.2）。一个「依次 fetch 三个资源」的探针
        #    光网络就吃掉好几秒，直接 TimeoutError，而栈里看不出是哪一步超时。
        self.sock = socket.create_connection((host, port), timeout=60)
        key = base64.b64encode(os.urandom(16)).decode()
        req = (
            f"GET {path} HTTP/1.1\r\n"
            f"Host: {host}:{port}\r\n"
            "Upgrade: websocket\r\nConnection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )
        self.sock.sendall(req.encode())
        buf = b""
        while b"\r\n\r\n" not in buf:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise ConnectionError("握手失败")
            buf += chunk
        if b"101" not in buf.split(b"\r\n")[0]:
            raise ConnectionError(f"握手被拒: {buf[:120]!r}")
        self.buf = buf.split(b"\r\n\r\n", 1)[1]
        self.seq = 0

    def _recv_exact(self, n: int) -> bytes:
        while len(self.buf) < n:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise ConnectionError("连接关闭")
            self.buf += chunk
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def send(self, payload: str) -> None:
        data = payload.encode()
        header = bytearray([0x81])
        mask = os.urandom(4)
        n = len(data)
        if n < 126:
            header.append(0x80 | n)
        elif n < 65536:
            header.append(0x80 | 126)
            header += struct.pack('>H', n)
        else:
            header.append(0x80 | 127)
            header += struct.pack('>Q', n)
        header += mask
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(data))
        self.sock.sendall(bytes(header) + masked)

    def recv(self) -> str:
        while True:
            b0, b1 = self._recv_exact(2)
            opcode = b0 & 0x0F
            length = b1 & 0x7F
            if length == 126:
                length = struct.unpack('>H', self._recv_exact(2))[0]
            elif length == 127:
                length = struct.unpack('>Q', self._recv_exact(8))[0]
            data = self._recv_exact(length)
            if opcode == 0x1:
                return data.decode('utf-8', 'ignore')
            if opcode == 0x8:
                raise ConnectionError("对端关闭")

    def call(self, method: str, params: dict | None = None) -> dict:
        self.seq += 1
        mid = self.seq
        self.send(json.dumps({"id": mid, "method": method, "params": params or {}}))
        while True:
            msg = json.loads(self.recv())
            if msg.get("id") == mid:
                return msg


def evaluate(ws_url_str: str, expr: str) -> int:
    ws = WS(ws_url_str)
    res = ws.call("Runtime.evaluate", {
        "expression": expr,
        "returnByValue": True,
        "awaitPromise": True,
    })
    result = res.get("result", {}).get("result", {})
    if "value" in result:
        value = result["value"]
        print(json.dumps(value, ensure_ascii=False, indent=2)
            if isinstance(value, (dict, list)) else str(value))
    else:
        print(json.dumps(res, ensure_ascii=False, indent=2)[:2000])
    return 0


def capture(ws_url_str: str, out: str) -> int:
    ws = WS(ws_url_str)
    res = ws.call("Page.captureScreenshot", {"format": "png"})
    data = res.get("result", {}).get("data")
    if not data:
        print(json.dumps(res, ensure_ascii=False)[:500])
        return 1
    path = Path(out)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(base64.b64decode(data))
    print(f"已写入 {path}（{path.stat().st_size} B）")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
