"""通用断言工具: 跟历史 baseline 兼容, 错误信息含 duration_ms."""

from typing import Any, Iterable


def assert_status(resp, expected: int, label: str = "") -> None:
    """断言 HTTP 状态码.

    Usage:
        assert_status(resp, 200, "create_session")
    """
    actual = resp.status_code
    if actual != expected:
        raise AssertionError(
            f"{label or 'HTTP'}: expected {expected}, got {actual} — "
            f"body={resp.text[:200]}"
        )


def assert_json_has(resp, *keys: str) -> None:
    """断言响应 JSON 含指定 keys (嵌套用 . 分隔)."""
    body = resp.json()
    for key in keys:
        parts = key.split(".")
        cur = body
        for p in parts:
            if not isinstance(cur, dict) or p not in cur:
                raise AssertionError(
                    f"missing key '{key}' in response: {body}"
                )
            cur = cur[p]


def assert_json_eq(resp, key: str, expected: Any) -> None:
    """断言响应 JSON 指定 key == expected."""
    body = resp.json()
    parts = key.split(".")
    cur = body
    for p in parts:
        cur = cur[p]
    if cur != expected:
        raise AssertionError(
            f"{key}: expected {expected!r}, got {cur!r} (full: {body})"
        )


def assert_in(needle: Any, haystack: Iterable[Any], label: str = "") -> None:
    """断言 needle in haystack, 错误信息含 label."""
    if needle not in haystack:
        items = list(haystack)[:10]
        raise AssertionError(
            f"{label or 'assert_in'}: {needle!r} not in {items}"
        )


def assert_eventually(condition, timeout_s: float = 5.0, interval_s: float = 0.1, label: str = ""):
    """轮询断言: 在 timeout_s 内, 每 interval_s 检查一次 condition().

    Usage:
        assert_eventually(lambda: session_is_active(id), timeout_s=3, label="session_active")
    """
    import time
    deadline = time.monotonic() + timeout_s
    last_exc = None
    while time.monotonic() < deadline:
        try:
            if condition():
                return True
        except AssertionError as e:
            last_exc = e
        time.sleep(interval_s)
    raise AssertionError(
        f"{label or 'assert_eventually'}: condition not met within {timeout_s}s "
        f"(last error: {last_exc})"
    )