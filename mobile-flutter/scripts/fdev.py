#!/usr/bin/env python3
"""千寻 Flutter 客户端 —— 构建/检查/测试的 Python 包装器。

为什么不让 shell 直接设环境变量再调 flutter：
  本机实测 `$env:PUB_*` 之类的赋值在某些执行环境里会被**静默丢弃**
  （设完读回来是空），而同样内容在 Python 的 `os.environ` 里是稳的。
  与其每次跟 shell 的变量作用域打架，不如在 Python 进程里把环境
  一次配好，再用 `subprocess` 调 flutter —— 子进程完整继承。

用法::

    python scripts\\fdev.py check          # analyze + test
    python scripts\\fdev.py analyze
    python scripts\\fdev.py test
    python scripts\\fdev.py apk             # 分 ABI release
    python scripts\\fdev.py apk --debug     # 单个 debug 包
    python scripts\\fdev.py clean
    python scripts\\fdev.py run -- --flavor dev
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FLUTTER = Path(r"E:\develop\tools\flutter\bin\flutter.bat")
JDK = Path(r"D:\programs\jdk-17.0.20.1+1")
ANDROID = Path(r"D:\programs\android-sdk")

# 缺少的组件会在这里被直接报出来，而不是让 flutter 抛一堆看不懂的错。
REQUIRED = [
    FLUTTER,
    JDK / "bin" / "java.exe",
    ANDROID / "platform-tools" / "adb.exe",
    ANDROID / "platforms" / "android-36",
]


def build_env() -> dict[str, str]:
    env = dict(os.environ)
    env["JAVA_HOME"] = str(JDK)
    env["ANDROID_HOME"] = str(ANDROID)
    env["ANDROID_SDK_ROOT"] = str(ANDROID)
    env["PATH"] = os.pathsep.join(
        [
            str(FLUTTER.parent),
            str(JDK / "bin"),
            str(ANDROID / "platform-tools"),
            str(ANDROID / "cmdline-tools" / "latest" / "bin"),
            env.get("PATH", ""),
        ]
    )
    return env


def preflight() -> list[str]:
    return [f"缺少组件：{p}" for p in REQUIRED if not p.exists()]


def run(args: list[str], *, cwd: Path = ROOT) -> int:
    """调 flutter，把输出原样透传。"""
    print(f"$ flutter {' '.join(args)}", flush=True)
    proc = subprocess.run(
        [str(FLUTTER), *args],
        cwd=str(cwd),
        env=build_env(),
        shell=False,
    )
    return proc.returncode


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    sub = ap.add_subparsers(dest="cmd", required=True)

    sub.add_parser("analyze")
    sub.add_parser("test")
    sub.add_parser("clean")
    sub.add_parser("check", help="analyze + test")

    apk = sub.add_parser("apk")
    apk.add_argument("--debug", action="store_true", help="构建单个 debug 包")
    apk.add_argument("--target", default="lib/main.dart")

    run_p = sub.add_parser("run")
    run_p.add_argument("rest", nargs=argparse.REMAINDER)

    args = ap.parse_args()

    missing = preflight()
    if missing:
        print("环境不齐，先修这些：", file=sys.stderr)
        for m in missing:
            print(f"  ✗ {m}", file=sys.stderr)
        return 2

    if args.cmd == "analyze":
        return run(["analyze"])
    if args.cmd == "test":
        return run(["test"])
    if args.cmd == "clean":
        return run(["clean"])
    if args.cmd == "check":
        rc = run(["analyze"])
        if rc != 0:
            return rc
        return run(["test"])
    if args.cmd == "apk":
        argv = ["build", "apk", "--target", args.target]
        if args.debug:
            argv += ["--debug"]
        else:
            argv += ["--release", "--split-per-abi"]
        return run(argv)
    if args.cmd == "run":
        rest = args.rest[1:] if args.rest and args.rest[0] == "--" else args.rest
        return run(["run", *rest])

    return 1


if __name__ == "__main__":
    sys.exit(main())
