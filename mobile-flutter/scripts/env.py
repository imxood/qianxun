#!/usr/bin/env python3
"""千寻 Flutter 客户端 —— 本机构建环境。

用法（PowerShell 里也只是薄薄一层胶水，逻辑全在本文件）::

    python scripts\\env.py --print        # 打印将要设置的环境变量
    python scripts\\env.py                # 输出一段可 eval 的命令，供当前 shell 注入

为什么不写成 .ps1 / .bat：
  - 跨平台、行为一致；
  - Windows 默认 ExecutionPolicy 禁止跑 .ps1；
  - 实测在 dot-source 的 .ps1 里设 $env:PUB_* 会被静默丢弃。

注意：子进程里设 os.environ **不会**影响父进程。所以本脚本做的是
「把变量算好、打印出来」，由调用方（人或胶水层）注入当前会话。
需要真正生效时，用 --emit 生成命令后执行::

    python scripts\\env.py --emit | Invoke-Expression
"""

from __future__ import annotations

import argparse
import os
import shutil
import sys
from pathlib import Path

# ── 本机组件位置 ──────────────────────────────────────────────────
FLUTTER_SDK = Path(r"E:\develop\tools\flutter")
JDK_HOME = Path(r"D:\programs\jdk-17.0.20.1+1")
ANDROID_HOME = Path(r"D:\programs\android-sdk")

# ── 国内镜像 ──────────────────────────────────────────────────────
# 本机实测 pub.dev 与 storage.flutter-io.cn 都可达，所以默认**不**切镜像。
# 网络受限时用 --mirror 打开。
MIRRORS = {
    "PUB_HOSTED_URL": "https://pub.flutter-io.cn",
    "FLUTTER_STORAGE_BASE_URL": "https://storage.flutter-io.cn",
}


def build_env(mirror: bool = False) -> dict[str, str]:
    """算出这个项目构建需要的环境变量。

    :param mirror: 是否切到国内镜像。
    """
    env: dict[str, str] = {
        "JAVA_HOME": str(JDK_HOME),
        "ANDROID_HOME": str(ANDROID_HOME),
        "ANDROID_SDK_ROOT": str(ANDROID_HOME),
    }
    if mirror:
        env.update(MIRRORS)

    # PATH：把需要的目录顶到前面，保留原有内容。
    extra = [
        FLUTTER_SDK / "bin",
        JDK_HOME / "bin",
        ANDROID_HOME / "platform-tools",
        ANDROID_HOME / "cmdline-tools" / "latest" / "bin",
        ANDROID_HOME / "emulator",
    ]
    parts = [str(p) for p in extra]
    current = os.environ.get("PATH", "")
    if current:
        parts.append(current)
    env["PATH"] = os.pathsep.join(parts)
    return env


def preflight(env: dict[str, str]) -> list[str]:
    """检查关键组件是否真的存在。返回问题列表（空 = 一切正常）。"""
    problems: list[str] = []
    if not (FLUTTER_SDK / "bin" / "flutter.bat").is_file():
        problems.append(f"Flutter SDK 缺失：{FLUTTER_SDK}")
    java = JDK_HOME / "bin" / "java.exe"
    if not java.is_file():
        problems.append(f"JDK 缺失：{JDK_HOME}")
    else:
        # 真的跑一次，别只看文件在不在——目录名对但二进制坏掉也常见。
        import subprocess

        try:
            out = subprocess.run(
                [str(java), "-version"],
                capture_output=True,
                text=True,
                timeout=30,
            )
            first = (out.stderr or out.stdout).splitlines()
            if out.returncode != 0 or not first:
                problems.append(f"JDK 无法执行：{first[:1]}")
        except Exception as exc:  # noqa: BLE001
            problems.append(f"JDK 无法执行：{exc}")

    adb = ANDROID_HOME / "platform-tools" / "adb.exe"
    if not adb.is_file():
        problems.append(f"adb 缺失：{adb}")
    platform = ANDROID_HOME / "platforms" / "android-36"
    if not platform.is_dir():
        problems.append(f"Android platform-36 缺失：{platform}")
    return problems


def main() -> int:
    ap = argparse.ArgumentParser(description="打印/校验千寻 Flutter 构建环境")
    ap.add_argument(
        "--emit",
        action="store_true",
        help="输出可被 shell eval 的 $env:NAME='VALUE' 赋值语句",
    )
    ap.add_argument("--mirror", action="store_true", help="切到国内镜像")
    ap.add_argument(
        "--check",
        action="store_true",
        help="检查组件是否齐备，缺什么报什么",
    )
    args = ap.parse_args()

    env = build_env(mirror=args.mirror)

    if args.emit:
        for key, value in env.items():
            # 单引号包裹，内部单引号按 PowerShell 规则转义。
            safe = value.replace("'", "''")
            print(f"$env:{key} = '{safe}'")
        return 0

    width = max(len(k) for k in env)
    for key, value in env.items():
        if key == "PATH":
            # PATH 太长，只显示我们加的前几段。
            shown = " ; ".join(value.split(os.pathsep)[:5]) + " ; …"
        else:
            shown = value
        print(f"{key:<{width}} = {shown}")

    print()
    print(f"flutter : {shutil.which('flutter', path=env['PATH']) or '<未找到>'}")
    print(f"adb     : {shutil.which('adb', path=env['PATH']) or '<未找到>'}")

    if args.check:
        problems = preflight(env)
        if problems:
            print()
            print("环境检查未通过：", file=sys.stderr)
            for p in problems:
                print(f"  ✗ {p}", file=sys.stderr)
            return 1
        print()
        print("环境检查通过。")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
