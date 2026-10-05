#!/usr/bin/env python3
"""把构建好的 APK 发布到本机 HTTP 服务目录（供手机浏览器下载）。

为什么需要它：MiniMax Code 手机端对附件是**内联预览**，有体积上限，
18MB 的 APK 也过不去。改成让电脑自己当文件服务器，手机用浏览器取。

用法::

    python scripts\\publish_apk.py                 # 同步最新构建并打印 URL
    python scripts\\publish_apk.py --port 8899
"""

from __future__ import annotations

import argparse
import hashlib
import http.server
import re
import shutil
import socketserver
import subprocess
import sys
import threading
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
APK_DIR = ROOT / "build" / "app" / "outputs" / "flutter-apk"
SERVE_DIR = Path(r"D:\Temp\qx-apk")
PORT = 8899

# 架构 → 一句话说明，给用户看。
LABEL = {
    "arm64-v8a": "现代手机（2017 年后）← 绝大多数人装这个",
    "armeabi-v7a": "老旧 32 位手机",
    "x86_64": "模拟器 / 少数平板",
}

# 从 pubspec 读版本号，别在这里手抄一份 —— 抄的那份一定会忘改，
# 结果就是新版包还叫旧名字，你在手机上分不清自己装的是哪个。
def app_version() -> str:
    text = (ROOT / "pubspec.yaml").read_text(encoding="utf-8")
    m = re.search(r"^version:\s*([0-9]+\.[0-9]+\.[0-9]+)", text, re.M)
    return m.group(1) if m else "0.0.0"


RENAMES = {
    "app-arm64-v8a-release.apk": "qianxun-{v}-arm64.apk",
    "app-armeabi-v7a-release.apk": "qianxun-{v}-armv7.apk",
    "app-x86_64-release.apk": "qianxun-{v}-x86_64.apk",
    "app-debug.apk": "qianxun-{v}-debug.apk",
}

# 旧版本号的包一并清掉：手机上从同一个 URL 下载时，最容易出的错就是
# 浏览器拿了一个旧文件（缓存、或上次下到一半），装上去却以为是最新的。
STALE_SUFFIXES = (".apk",)


def sync() -> list[Path]:
    """把最新构建拷到服务目录。返回拷过去的文件。"""
    if not APK_DIR.is_dir():
        print(f"没有构建产物：{APK_DIR}", file=sys.stderr)
        return []
    version = app_version()
    SERVE_DIR.mkdir(parents=True, exist_ok=True)

    for old in SERVE_DIR.glob("*.apk"):
        old.unlink()
        print(f"  清掉旧包 {old.name}")

    out: list[Path] = []
    for src_name, dst_tpl in RENAMES.items():
        src = APK_DIR / src_name
        if not src.is_file():
            continue
        dst = SERVE_DIR / dst_tpl.format(v=version)
        shutil.copy2(src, dst)
        out.append(dst)
    return out


def sha256(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def lan_ips() -> list[str]:
    """列出本机可用的局域网 IPv4（跳过回环 / 虚拟网卡 / EasyTier）。

    ⚠️ `ipconfig` 在中文 Windows 上输出是 **GBK**，按 UTF-8 解会抛
    UnicodeDecodeError 然后连 `stdout` 都拿不到（subprocess 的读线程直接
    死了，capture_output 返回 None）。所以显式用 gbk 解码。
    """
    ips: list[str] = []
    try:
        proc = subprocess.run(
            ["ipconfig"], capture_output=True, timeout=15
        )
    except Exception:  # noqa: BLE001
        return ips
    # 逐个编码试，GBK 覆盖简体中文 Windows；失败就用 replace 兜底。
    out = ""
    for enc in ("gbk", "utf-8"):
        try:
            out = proc.stdout.decode(enc)
            break
        except UnicodeDecodeError:
            continue
    else:
        out = (proc.stdout or b"").decode("utf-8", "replace")

    skip = (
        "VirtualBox",
        "VMware",
        "vEthernet",
        "Loopback",
        "Meta",
        "蓝牙",
        "BTH",
        "TUN",
        "VPN",
        "Wintun",
    )
    # 198.18.0.0/15 是基准测试保留段，多半是 TUN/VPN 虚拟网卡，
    # 手机访问不到，也不该出现在给用户的地址列表里。
    def _usable(ip: str) -> bool:
        if not ip or ip.startswith("127.") or ip.startswith("169.254."):
            return False
        if ip.startswith("198.18.") or ip.startswith("198.19."):
            return False
        return not any(s.lower() in current.lower() for s in skip)

    current = ""
    for line in out.splitlines():
        if not line.strip():
            current = ""
            continue
        if not line.startswith((" ", "\t")):
            current = line.rstrip(":").strip()
        elif "IPv4" in line:
            ip = line.split(":", 1)[-1].strip()
            if _usable(ip):
                ips.append(ip)
    # 去重保序
    seen: set[str] = set()
    return [i for i in ips if not (i in seen or seen.add(i))]


class Handler(http.server.SimpleHTTPRequestHandler):
    def guess_type(self, path: str) -> str:
        if path.endswith(".apk"):
            return "application/vnd.android.package-archive"
        return super().guess_type(path)

    def end_headers(self) -> None:
        # 换包后手机侧别拿旧的。
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def log_message(self, fmt: str, *a) -> None:  # noqa: ANN001
        sys.stderr.write("%s %s\n" % (self.address_string(), fmt % a))


def serve(port: int) -> None:
    os_chdir = SERVE_DIR
    import os

    os.chdir(str(os_chdir))
    socketserver.TCPServer.allow_reuse_address = True
    with socketserver.ThreadingTCPServer(("0.0.0.0", port), Handler) as httpd:
        print(f"serving {SERVE_DIR} on 0.0.0.0:{port}", flush=True)
        thread = threading.Thread(target=httpd.serve_forever, daemon=True)
        thread.start()
        try:
            thread.join()
        except KeyboardInterrupt:
            httpd.shutdown()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=PORT)
    ap.add_argument("--no-serve", action="store_true", help="只同步，不起服务")
    args = ap.parse_args()

    files = sync()
    if not files:
        return 1

    print("已发布：")
    for f in sorted(files, key=lambda x: -x.stat().st_size):
        mb = f.stat().st_size / 1024 / 1024
        arch = next((k for k in LABEL if k in f.name), "")
        print(f"  {f.name:<28} {mb:5.1f} MB  {LABEL.get(arch, '')}")
        print(f"      sha256 {sha256(f)[:32]}…")

    ips = lan_ips()
    print("\n下载地址（手机浏览器打开）：")
    for ip in ips or ["127.0.0.1"]:
        print(f"  http://{ip}:{args.port}/")
    if not ips:
        print("  ⚠️ 没找到局域网地址，手机可能访问不到")

    if args.no_serve:
        return 0
    print(f"\n服务已在后台就绪（Ctrl-C 结束）。目录：{SERVE_DIR}")
    serve(args.port)
    return 0


if __name__ == "__main__":
    sys.exit(main())
