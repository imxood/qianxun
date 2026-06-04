#!/usr/bin/env python3
# 千寻 daemon 统一工作流 (Stage 12)
#
# 单脚本, 4 模式, 取代 dev.py / release.py / build.rs:
#   - 默认 (无 flag)         = dev:        启 vite (后台) + cargo run daemon (前台)
#   - --release              = release:    pnpm build + cargo build --release + 跑 release 二进制
#   - --build                = build debug: pnpm build + cargo build + 退出 (CI 用)
#   - --release --build      = build release: pnpm build + cargo build --release + 退出
#
# 通用修饰:
#   --port 23910            自定义 daemon 端口
#   --no-vite               不启 vite (dev 模式单跑 daemon, 假设 vite 已在跑)
#   --skip-build            跳过 cargo/pnpm build (assume 已 build)
#   --ui-dist <path>        release 模式覆盖 UI dist 路径 (default: qianxun/src/daemon/ui/build)
#
# vite 自带 watch (dev mode 改 svelte 自动 HMR) + 自带 build (release),
# py 脚本只负责 orchestrate, 不重实现.
#
# 人类易读本地时间日志: HH:MM:SS.mmm (本地时区) + 颜色 + emoji 状态指示.
#
# 依赖: Python 3.8+ stdlib only.
# 跨平台: Windows (CREATE_NEW_PROCESS_GROUP + taskkill) + POSIX (process group).

import argparse
import os
import signal
import subprocess
import sys
import time
from datetime import datetime
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
UI_DIR = REPO_ROOT / "qianxun" / "src" / "daemon" / "ui"
UI_DIST_DEFAULT = UI_DIR / "build"
BIN_DEBUG = REPO_ROOT / "target" / "debug" / ("qx.exe" if sys.platform == "win32" else "qx")
BIN_RELEASE = REPO_ROOT / "target" / "release" / ("qx.exe" if sys.platform == "win32" else "qx")

# 系统代理 (loopback 不该走代理, 调试时干扰测试) — spawn 子进程前 unset
PROXY_VARS = ("HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy",
              "ALL_PROXY", "all_proxy", "NO_PROXY", "no_proxy")

# ANSI 颜色 (Windows Terminal / 现代 terminal 都支持; 老 cmd 退化)
C_RESET = "\033[0m"
C_DIM = "\033[2m"
C_BOLD = "\033[1m"
C_RED = "\033[31m"
C_GREEN = "\033[32m"
C_YELLOW = "\033[33m"
C_BLUE = "\033[34m"
C_MAGENTA = "\033[35m"
C_CYAN = "\033[36m"


def now() -> str:
    """人类易读的本地时间戳: HH:MM:SS.mmm"""
    return datetime.now().strftime("%H:%M:%S.") + f"{datetime.now().microsecond // 1000:03d}"


def log(level: str, msg: str, *parts: str) -> None:
    """统一日志: 时间 + level + 消息 + 可选子部分.

    levels: info, ok, warn, err, step
    """
    colors = {
        "info":  C_CYAN,
        "ok":    C_GREEN,
        "warn":  C_YELLOW,
        "err":   C_RED,
        "step":  C_BOLD + C_MAGENTA,
    }
    icons = {
        "info":  "ℹ",
        "ok":    "✓",
        "warn":  "⚠",
        "err":   "✗",
        "step":  "▸",
    }
    c = colors.get(level, C_RESET)
    icon = icons.get(level, " ")
    head = f"{C_DIM}{now()}{C_RESET} {c}{icon} {level:>4}{C_RESET}  {msg}"
    if parts:
        rest = " ".join(f"{C_DIM}{p}{C_RESET}" for p in parts)
        print(f"{head}  {rest}", flush=True)
    else:
        print(head, flush=True)


def step(n: int, total: int, msg: str) -> None:
    log("step", f"[{n}/{total}] {msg}")


def die(msg: str, code: int = 1) -> None:
    log("err", msg)
    sys.exit(code)


def is_port_in_use(port: int) -> bool:
    import socket
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.settimeout(0.5)
        try:
            s.connect(("127.0.0.1", port))
            return True
        except (ConnectionRefusedError, socket.timeout, OSError):
            return False


def wait_for_port(port: int, timeout: float, label: str) -> bool:
    """等端口起来, 试 IPv4 + IPv6 + 'localhost' (Windows vite 默认 bind localhost)."""
    import socket
    deadline = time.time() + timeout
    while time.time() < deadline:
        for host in ("127.0.0.1", "::1", "localhost"):
            family = socket.AF_INET6 if ":" in host else socket.AF_INET
            with socket.socket(family, socket.SOCK_STREAM) as s:
                s.settimeout(0.3)
                try:
                    s.connect((host, port))
                    log("ok", f"{label}:{port} up (via {host})")
                    return True
                except (ConnectionRefusedError, socket.timeout, OSError):
                    pass
        time.sleep(0.5)
    log("err", f"{label}:{port} timeout after {timeout}s")
    return False


def make_clean_env(extra: dict | None = None) -> dict:
    """构造子进程环境: 不带代理 + 合并 extra."""
    env = os.environ.copy()
    for v in PROXY_VARS:
        env.pop(v, None)
    if extra:
        env.update(extra)
    return env


def run_step(label: str, cmd: list[str], cwd: Path, env_extra: dict | None = None,
             timeout: float | None = None) -> bool:
    """跑一个 build step, 实时流式输出, 失败返 False."""
    log("info", f"$ {C_BOLD}{' '.join(cmd)}{C_RESET}  (cwd: {cwd.name})")
    t0 = time.time()
    try:
        proc = subprocess.Popen(
            cmd, cwd=cwd,
            env=make_clean_env(env_extra),
            stdout=None,  # inherit (实时流)
            stderr=subprocess.STDOUT,  # merge
            stdin=None,
        )
        rc = proc.wait(timeout=timeout)
    except KeyboardInterrupt:
        log("warn", "interrupted, terminating step...")
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
        raise
    dt = time.time() - t0
    if rc == 0:
        log("ok", f"{label} done in {dt:.1f}s")
        return True
    log("err", f"{label} failed (exit {rc}) in {dt:.1f}s")
    return False


def spawn_bg(cmd: list[str], cwd: Path, env_extra: dict | None = None,
             creationflags: int = 0) -> subprocess.Popen:
    """后台 spawn 子进程 (stdout inherit)."""
    env = make_clean_env(env_extra)
    return subprocess.Popen(
        cmd, cwd=cwd, env=env,
        stdout=None, stderr=subprocess.STDOUT, stdin=None,
        creationflags=creationflags,
        start_new_session=(sys.platform != "win32"),
    )


def kill_tree(proc: subprocess.Popen | None) -> None:
    if proc is None or proc.poll() is not None:
        return
    log("info", f"killing pid {proc.pid} (tree)...")
    try:
        if sys.platform == "win32":
            subprocess.run(
                ["taskkill", "/F", "/T", "/PID", str(proc.pid)],
                capture_output=True, timeout=5,
            )
        else:
            os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
    except (ProcessLookupError, OSError, subprocess.TimeoutExpired):
        pass


# ─── 模式 dispatch ────────────────────────────────────────────────────

def mode_dev(args) -> int:
    """dev 模式: 后台启 vite, 前台 cargo run daemon, daemon 反代 /ui → vite."""
    port = args.port
    if is_port_in_use(port):
        return die(f"daemon 端口 {port} 已被占. 用 --port 改或关掉旧 daemon.")
    vite_port = int(args.ui_dev.rsplit(":", 1)[1].rstrip("/"))
    if not args.no_vite and is_port_in_use(vite_port):
        return die(f"vite 端口 {vite_port} 已被占. 用 --ui-dev 改 URL 或 --no-vite (假设 vite 已在跑).")

    log("step", f"模式 = DEV (debug + vite watch + 反代)")
    log("info", f"daemon 端口 : {port} (--ui-dev → {args.ui_dev})")
    log("info", f"vite 端口   : {vite_port}  (SvelteKit paths.base='/ui')")
    log("info", f"浏览器入口  : {C_BOLD}http://127.0.0.1:{port}/ui{C_RESET}")
    log("info", f"vite HMR 备用: http://127.0.0.1:{vite_port}/ui")
    log("info", f"Ctrl-C 优雅关闭 (killtree 两个子进程)")

    # cargo + pnpm 跨平台 spawn flag
    creationflags = subprocess.CREATE_NEW_PROCESS_GROUP if sys.platform == "win32" else 0
    pnpm_cmd = "pnpm.cmd" if sys.platform == "win32" else "pnpm"

    vite_proc: subprocess.Popen | None = None
    daemon_proc: subprocess.Popen | None = None

    try:
        # 1. 启 vite (后台)
        if not args.no_vite:
            log("step", "启 vite dev server (后台)")
            vite_proc = spawn_bg(
                [pnpm_cmd, "run", "dev"],
                cwd=UI_DIR,
                creationflags=creationflags,
            )
            if not wait_for_port(vite_port, timeout=20.0, label="vite"):
                kill_tree(vite_proc)
                return die("vite 启动失败, 退出.")
        else:
            log("info", f"--no-vite 设了, 假设 vite 已在 {args.ui_dev} 跑")

        # 2. cargo run daemon (前台, 用户看 Rust 日志)
        log("step", "启 cargo run daemon (前台)")
        cargo_env = {"QIANXUN_SKIP_UI_BUILD": "1"}  # 现在 build.rs 已删, 没必要, 但留着无害
        daemon_cmd = [
            "cargo", "run",
            "-p", "qianxun", "--bin", "qx",
            "--", "--daemon",
            "--port", str(port),
            "--ui-dev", args.ui_dev,
        ]
        daemon_proc = spawn_bg(
            daemon_cmd, cwd=REPO_ROOT,
            env_extra=cargo_env,
            creationflags=creationflags,
        )

        if not wait_for_port(port, timeout=90.0, label="daemon"):
            kill_tree(daemon_proc)
            kill_tree(vite_proc)
            return die("daemon 启动失败, 退出.")

        log("ok", f"全部就绪 → http://127.0.0.1:{port}/ui")
        log("info", f" 改 svelte 后浏览器 Cmd-R (vite 自动 watch)")

        # 3. 等子进程退出 / Ctrl-C
        while True:
            time.sleep(0.5)
            vp = vite_proc.poll() if vite_proc else None
            dp = daemon_proc.poll()
            if vp is not None or dp is not None:
                log("warn", f"子进程退出 (vite={vp}, daemon={dp}), 关另一个")
                if vp is None:
                    kill_tree(vite_proc)
                if dp is None:
                    kill_tree(daemon_proc)
                return 1 if (vp != 0 or dp != 0) else 0

    except KeyboardInterrupt:
        log("info", "Ctrl-C 收到, 优雅关子进程...")
        kill_tree(daemon_proc)
        kill_tree(vite_proc)
        return 0


def mode_release(args) -> int:
    """release 模式: pnpm build + cargo build --release + 跑 release 二进制."""
    port = args.port
    if not args.build and is_port_in_use(port):
        return die(f"daemon 端口 {port} 已被占. 用 --port 改.")

    log("step", f"模式 = RELEASE (--release, {'只 build 不跑' if args.build else 'build + 跑'})")
    log("info", f"产物: {BIN_RELEASE}")
    log("info", f"UI:   {args.ui_dist}")
    if not args.build:
        log("info", f"入口: {C_BOLD}http://127.0.0.1:{port}/ui{C_RESET}")

    total = 3 if not args.skip_build else 1
    n = 0

    if not args.skip_build:
        # 1. pnpm build
        n += 1
        pnpm_cmd = "pnpm.cmd" if sys.platform == "win32" else "pnpm"
        if not run_step(f"step {n}/{total}: pnpm build", [pnpm_cmd, "run", "build"], UI_DIR):
            return 1

        # 2. cargo build --release
        n += 1
        if not run_step(
            f"step {n}/{total}: cargo build --release",
            ["cargo", "build", "--release", "-p", "qianxun", "--bin", "qx"],
            REPO_ROOT,
            timeout=600.0,  # release 编译 1-3min
        ):
            return 1

    # 验证产物
    if not BIN_RELEASE.exists():
        return die(f"产物不存在: {BIN_RELEASE}")
    if not (Path(args.ui_dist) / "index.html").exists():
        return die(f"UI dist 缺 index.html: {args.ui_dist}")

    if args.build:
        log("ok", f"build 完成, 跑 daemon: {BIN_RELEASE} --daemon --port {port} --ui-dist {args.ui_dist}")
        return 0

    # 3. 跑 release 二进制 (前台)
    n += 1
    log("step", f"step {n}/{total}: 跑 release daemon")
    log("info", f"按 Ctrl-C 退出")
    try:
        rc = subprocess.call(
            [str(BIN_RELEASE), "--daemon", "--port", str(port), "--ui-dist", str(args.ui_dist)],
            cwd=REPO_ROOT,
        )
        return rc
    except KeyboardInterrupt:
        return 0


def main() -> int:
    p = argparse.ArgumentParser(
        prog="run.py",
        description="千寻 daemon 统一工作流 (dev / release / build)",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )
    g = p.add_argument_group("模式 (互斥-ish)")
    g.add_argument("--release", action="store_true",
                   help="release 模式: 编译 + 跑 release 优化版 (默认 dev)")
    g.add_argument("--build", action="store_true",
                   help="只 build 不跑 (跟 --release 配 → build release; 不配 → build debug)")
    g = p.add_argument_group("通用")
    g.add_argument("--port", type=int, default=23900, help="daemon 端口 (default 23900)")
    g.add_argument("--ui-dev", default="http://127.0.0.1:5174",
                   help="dev 模式 vite URL (default http://127.0.0.1:5174)")
    g.add_argument("--no-vite", action="store_true",
                   help="dev 模式不启 vite (假设 vite 已在跑, 跟 daemon 反代联调)")
    g.add_argument("--ui-dist", default=str(UI_DIST_DEFAULT),
                   help="release 模式 UI 静态 dist 路径")
    g.add_argument("--skip-build", action="store_true",
                   help="跳过 pnpm + cargo build (assume 已 build, 直接跑)")
    args = p.parse_args()

    # 模式分发
    if args.build or args.release:
        return mode_release(args)
    return mode_dev(args)


if __name__ == "__main__":
    sys.exit(main())
