"""计时工具: 提供 ctxmanager 风格的计时器, 自动记录 elapsed_ms."""

import time
from contextlib import contextmanager
from typing import Iterator


@contextmanager
def timed(label: str = "") -> Iterator[dict]:
    """计时 ctxmanager.

    Usage:
        with timed("POST /v1/chat/session") as t:
            r = client.post(...)
        assert t["elapsed_ms"] < 1000
    """
    t = {"label": label, "elapsed_ms": 0, "start": time.monotonic()}
    try:
        yield t
    finally:
        t["elapsed_ms"] = int((time.monotonic() - t["start"]) * 1000)