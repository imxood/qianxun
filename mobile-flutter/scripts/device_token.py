"""从模拟器已 root 的 /data/data 下读 App 的 qxToken，供探针复用。

背景
----
release 包不是 debuggable，`run-as` 用不了。但**模拟器可以 `adb root`**
（用户版真机不行）。这让原型探针不必让用户手工复制 64 位 hex。

安全性
------
只读。token 只在内存里传给子进程，不落盘、不打印、不写进 git。
输出里只报长度与来源文件。
"""
from __future__ import annotations

import os
import re
import subprocess
import sys
from pathlib import Path

PKG = "com.qianxun.qianxun_mobile"
ADB = Path(r"D:\programs\android-sdk\platform-tools\adb.exe")


def shell(serial: str, command: str, timeout: int = 25) -> str:
    proc = subprocess.run(
        [str(ADB), "-s", serial, "shell", command],
        capture_output=True, timeout=timeout,
    )
    return (proc.stdout or b"").decode("utf-8", "ignore")


def first_device() -> str | None:
    out = (subprocess.run(
        [str(ADB), "devices"], capture_output=True, timeout=20,
    ).stdout or b"").decode("utf-8", "ignore")
    for line in out.splitlines():
        if line.startswith("emulator-") and line.endswith("\tdevice"):
            return line.split("\t")[0]
    return None


def main() -> int:
    serial = first_device()
    if serial is None:
        print("没有在线模拟器。")
        return 1
    print(f"设备: {serial}", file=sys.stderr)

    base = f"/data/data/{PKG}/shared_prefs"
    listing = shell(serial, f"ls {base}")
    if "Permission denied" in listing:
        print("设备未 root。模拟器上先跑 `adb root`。", file=sys.stderr)
        return 1
    names = re.findall(r"(\S+\.xml)", listing)
    if not names:
        print(f"没有 prefs 文件：{listing.strip()[:200]}", file=sys.stderr)
        return 1

    for name in names:
        raw = shell(serial, f"cat {base}/{name}")
        # shared_preferences 的值是 XML 转义过的 JSON，所以里面是 &quot; 不是 "。
        # 凭据有两个可能位置：
        #   1. qxToken 字段（配对后补写）
        #   2. 配对链接 url 里的 ?token=（**实测本机是这个**）
        # 只认 &quot; 形式，直接对原始 JSON 跑正则会一个都匹配不到。
        for pattern in (
            r'qxToken&quot;:&quot;([0-9a-fA-F]{32,})&quot;',
            r'[?&]token=([0-9a-fA-F]{32,})',
        ):
            found = re.findall(pattern, raw)
            for value in found:
                if value.lower() in {"null", "0" * len(value)}:
                    continue
                where = "qxToken 字段" if "qxToken" in pattern else "配对链接 url"
                print(f"{name} · {where} · {len(value)} 位", file=sys.stderr)
                print(value)
                return 0

    print("prefs 里没有可用凭据（可能尚未配对）。", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
