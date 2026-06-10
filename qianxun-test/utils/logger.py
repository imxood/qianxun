"""测试日志格式化.

输出格式 (跟历史 baseline 一致):
    [qx-test] INFO  POST /v1/chat/session → 200 OK (47ms)
    [qx-test] OK    test_daemon_001_concurrent_sessions (13.7s)
    [qx-test] ERR   test_daemon_002_large_prompt_stream → AssertionError
"""

import sys
from datetime import datetime


_PREFIX = "[qx-test]"


def _stamp() -> str:
    return datetime.now().strftime("%H:%M:%S.%f")[:-3]


def info(msg: str) -> None:
    print(f"{_stamp()} {_PREFIX} INFO  {msg}", file=sys.stderr)


def ok(msg: str) -> None:
    print(f"{_stamp()} {_PREFIX} OK    {msg}", file=sys.stderr)


def err(msg: str) -> None:
    print(f"{_stamp()} {_PREFIX} ERR   {msg}", file=sys.stderr)


def warn(msg: str) -> None:
    print(f"{_stamp()} {_PREFIX} WARN  {msg}", file=sys.stderr)


def section(title: str) -> None:
    """打印测试 section 标题 (用于复杂测试的分段输出)."""
    print(f"\n{_stamp()} {_PREFIX} ==== {title} ====", file=sys.stderr)


def http(method: str, path: str, status: int, elapsed_ms: int) -> None:
    """HTTP 请求单行日志."""
    info(f"{method} {path} → {status} ({elapsed_ms}ms)")