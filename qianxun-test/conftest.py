"""pytest conftest.py — qianxun-test 全局 fixture.

职责:
1. session-scoped: 启动 daemon binary (cargo run -- --daemon --port 23900)
2. session-scoped: 捕获 daemon stderr 中的 admin password ([admin-auth]   <pwd>)
3. session-scoped: 提供已鉴权的 `client` fixture (DaemonClient)
4. session-scoped: 测试结束 kill daemon

设计取舍:
- daemon 用 subprocess 启动, 不依赖外部已运行的 daemon (e2e 隔离性)
- daemon 临时工作目录: QIANXUN_HOME env var 指向 tmp/, 不污染 ~/.qianxun
- admin.cred 首次启动打印 password 到 stderr, conftest 用 regex 提取
- session 失败时打印 daemon 日志末尾 50 行, 方便排查
"""

import os
import re
import shutil
import signal
import subprocess
import tempfile
import time
from pathlib import Path
from typing import Iterator

import pytest

from utils import logger
from utils.daemon_client import ApiError, DaemonClient


# ─── Config ─────────────────────────────────────────────────

DAEMON_PORT = int(os.environ.get("QIANXUN_TEST_PORT", "23900"))
DAEMON_HOST = "127.0.0.1"
DAEMON_BASE_URL = f"http://{DAEMON_HOST}:{DAEMON_PORT}"
DAEMON_USER = "admin"

# cargo run -- --daemon 启动命令模板
# 用预先 cargo build 后的 binary 路径, 避免测试启动时还要编译
WORKSPACE_ROOT = Path(__file__).resolve().parent.parent
DAEMON_BIN = WORKSPACE_ROOT / "target" / "debug" / "qianxun.exe"

# daemon stderr 中提取 password 的正则 (匹配 "[admin-auth]   <password>" 行)
_PASSWORD_RE = re.compile(r"\[admin-auth\]\s+([A-Za-z0-9_-]{20,32})")
# daemon 启动成功的标志 (HTTP 200 from /v1/system/health)
_HEALTH_URL = f"{DAEMON_BASE_URL}/v1/system/health"
# daemon 启动超时
DAEMON_STARTUP_TIMEOUT_S = 30


# ─── Fixtures ───────────────────────────────────────────────


@pytest.fixture(scope="session")
def qianxun_home() -> Iterator[Path]:
    """临时 qianxun home 目录 (env QIANXUN_HOME).

    测完删除, 不污染用户 ~/.qianxun.
    """
    tmp = Path(tempfile.mkdtemp(prefix="qianxun-test-"))
    logger.info(f"using temp QIANXUN_HOME: {tmp}")
    old_home = os.environ.get("QIANXUN_HOME")
    os.environ["QIANXUN_HOME"] = str(tmp)
    try:
        yield tmp
    finally:
        if old_home is None:
            os.environ.pop("QIANXUN_HOME", None)
        else:
            os.environ["QIANXUN_HOME"] = old_home
        # 清理临时目录
        try:
            shutil.rmtree(tmp, ignore_errors=True)
        except Exception as e:
            logger.warn(f"failed to cleanup {tmp}: {e}")


@pytest.fixture(scope="session")
def daemon_process(qianxun_home: Path) -> Iterator[subprocess.Popen]:
    """session-scoped daemon 进程.

    启动 cargo run -- --daemon --port <PORT>, 等 health check 通过,
    提取 admin password, 测试结束后 kill.
    """
    if not DAEMON_BIN.exists():
        # 尝试 fallback: cargo run -- --daemon
        # 但测试应优先用预编译 binary, 加快启动
        logger.warn(f"daemon binary not found at {DAEMON_BIN}, falling back to cargo run")

    cmd = [
        str(DAEMON_BIN) if DAEMON_BIN.exists() else "cargo",
        *(["run", "--", "--daemon", "--port", str(DAEMON_PORT)] if not DAEMON_BIN.exists()
          else ["--daemon", "--port", str(DAEMON_PORT)]),
    ]

    logger.info(f"starting daemon: {' '.join(cmd)}")
    proc = subprocess.Popen(
        cmd,
        cwd=str(WORKSPACE_ROOT),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
        env={**os.environ, "QIANXUN_HOME": str(qianxun_home)},
    )

    # 等 daemon ready (poll health endpoint)
    deadline = time.monotonic() + DAEMON_STARTUP_TIMEOUT_S
    password: str | None = None
    last_stderr_lines: list[str] = []

    # 后台线程读 stderr, 捕获 password
    import threading
    stderr_buffer: list[str] = []

    def _read_stderr():
        assert proc.stderr is not None
        for line in proc.stderr:
            stderr_buffer.append(line.rstrip())
            m = _PASSWORD_RE.search(line)
            if m:
                nonlocal password
                password = m.group(1)

    reader = threading.Thread(target=_read_stderr, daemon=True)
    reader.start()

    ready = False
    while time.monotonic() < deadline:
        # 用 requests 试 health (避免依赖 client fixture 还未创建)
        try:
            import requests
            r = requests.get(_HEALTH_URL, timeout=1.0)
            if r.status_code == 200:
                ready = True
                break
        except Exception:
            pass
        time.sleep(0.2)

    if not ready:
        # 失败: 打印 stderr, kill 进程
        proc.kill()
        logger.err(f"daemon failed to start within {DAEMON_STARTUP_TIMEOUT_S}s")
        logger.err("last 50 lines of stderr:")
        for line in stderr_buffer[-50:]:
            logger.err(f"  {line}")
        raise RuntimeError(f"daemon startup timeout (port {DAEMON_PORT})")

    if not password:
        proc.kill()
        logger.err("daemon started but no admin password captured")
        logger.err("stderr:")
        for line in stderr_buffer[-30:]:
            logger.err(f"  {line}")
        raise RuntimeError("daemon admin password not captured from stderr")

    logger.info(f"daemon ready on port {DAEMON_PORT} (password len={len(password)})")
    # 把 password 存到 proc 对象的属性, 后面 fixture 用
    proc._qx_password = password  # type: ignore[attr-defined]

    try:
        yield proc
    finally:
        logger.info("killing daemon")
        try:
            proc.terminate()
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait(timeout=2)
        except Exception as e:
            logger.warn(f"error killing daemon: {e}")


@pytest.fixture(scope="session")
def client(daemon_process: subprocess.Popen) -> Iterator[DaemonClient]:
    """session-scoped 已鉴权 DaemonClient."""
    password = daemon_process._qx_password  # type: ignore[attr-defined]
    c = DaemonClient(DAEMON_BASE_URL)
    c.login(DAEMON_USER, password)
    logger.info("DaemonClient ready (authenticated)")
    try:
        yield c
    finally:
        c.close()


# ─── Hooks ──────────────────────────────────────────────────


def pytest_configure(config):
    """注册自定义 markers, 避免 PytestUnknownMarkWarning."""
    config.addinivalue_line(
        "markers",
        "timeout(seconds): set per-test timeout (需要 pytest-timeout, 未装时仅 warning)",
    )


def pytest_sessionfinish(session, exitstatus):
    """session 结束时打印总耗时."""
    logger.info(f"pytest session finished, exit status: {exitstatus}")


@pytest.hookimpl(tryfirst=True, hookwrapper=True)
def pytest_runtest_makereport(item, call):
    """测试失败时打印 daemon 进程状态 (如果还活着)."""
    outcome = yield
    rep = outcome.get_result()
    if rep.failed and "daemon_process" in item.fixturenames:
        proc = item.funcargs.get("daemon_process")
        if proc and proc.poll() is None:
            logger.warn(f"daemon still running (pid={proc.pid}) after test failure")