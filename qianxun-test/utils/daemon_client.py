"""daemon HTTP 客户端 (带 JWT 鉴权).

设计要点:
- 单例 client, daemon 启动时获取 JWT, 后续请求自动带 Authorization header
- 支持 SSE 流式响应解析 (yield 事件字典)
- 错误分类: 4xx/5xx 抛 ApiError 带 status + body
- session 复用: connection pool, requests.Session 内部管理
"""

from typing import Any, Iterator, Optional

import requests

from utils import logger


class ApiError(Exception):
    """daemon API 错误 (HTTP 4xx/5xx)."""

    def __init__(self, status: int, body: str, path: str):
        self.status = status
        self.body = body
        self.path = path
        super().__init__(f"HTTP {status} on {path}: {body[:200]}")


class DaemonClient:
    """daemon HTTP 客户端.

    Usage:
        client = DaemonClient("http://127.0.0.1:23900")
        client.login("admin", password)
        sid = client.create_session()
        events = client.stream_prompt(sid, messages=[...])
    """

    def __init__(self, base_url: str, timeout: float = 30.0):
        self.base_url = base_url.rstrip("/")
        self.timeout = timeout
        self._session = requests.Session()
        self._jwt: Optional[str] = None

    # ── Auth ──

    def login(self, username: str, password: str) -> dict:
        """登录拿 JWT (写 ~/.qianxun/admin.cred 时 daemon 首启打印 password)."""
        resp = self._session.post(
            f"{self.base_url}/v1/auth/login",
            json={"username": username, "password": password},
            timeout=self.timeout,
        )
        if resp.status_code != 200:
            raise ApiError(resp.status_code, resp.text, "/v1/auth/login")
        body = resp.json()
        self._jwt = body.get("token")
        if not self._jwt:
            raise ApiError(200, "no token in response", "/v1/auth/login")
        logger.info(f"login OK, jwt len={len(self._jwt)}")
        return body

    def _headers(self) -> dict:
        if not self._jwt:
            raise ApiError(0, "not authenticated", "")
        return {"Authorization": f"Bearer {self._jwt}"}

    # ── Sessions ──

    def create_session(self, model: Optional[str] = None) -> str:
        """创建 session, 返 session_id."""
        body: dict = {}
        if model:
            body["model"] = model
        resp = self._session.post(
            f"{self.base_url}/v1/chat/session",
            json=body,
            headers=self._headers(),
            timeout=self.timeout,
        )
        if resp.status_code != 200:
            raise ApiError(resp.status_code, resp.text, "/v1/chat/session")
        return resp.json()["session_id"]

    def list_sessions(self) -> list[dict]:
        resp = self._session.get(
            f"{self.base_url}/v1/chat/sessions",
            headers=self._headers(),
            timeout=self.timeout,
        )
        if resp.status_code != 200:
            raise ApiError(resp.status_code, resp.text, "/v1/chat/sessions")
        return resp.json().get("sessions", [])

    def get_session(self, session_id: str) -> dict:
        resp = self._session.get(
            f"{self.base_url}/v1/chat/session/{session_id}",
            headers=self._headers(),
            timeout=self.timeout,
        )
        if resp.status_code != 200:
            raise ApiError(resp.status_code, resp.text, f"/v1/chat/session/{session_id}")
        return resp.json()

    def delete_session(self, session_id: str) -> None:
        resp = self._session.delete(
            f"{self.base_url}/v1/chat/session/{session_id}",
            headers=self._headers(),
            timeout=self.timeout,
        )
        if resp.status_code != 200:
            raise ApiError(resp.status_code, resp.text, f"DELETE /v1/chat/session/{session_id}")

    def stream_prompt(
        self,
        session_id: str,
        messages: list[dict],
        timeout: float = 120.0,
    ) -> Iterator[dict]:
        """流式发送 prompt, yield SseEvent dict.

        messages: [{"role": "user", "content": "..."}, ...]
        """
        resp = self._session.post(
            f"{self.base_url}/v1/chat/session/{session_id}/prompt",
            json={"messages": messages},
            headers=self._headers(),
            stream=True,
            timeout=timeout,
        )
        if resp.status_code != 200:
            raise ApiError(resp.status_code, resp.text, f"prompt/{session_id}")
        # SSE 格式: data: {...}\n\n
        for line in resp.iter_lines(decode_unicode=True):
            if not line or not line.startswith("data: "):
                continue
            payload = line[6:]  # 去掉 "data: " 前缀
            if payload == "[DONE]":
                break
            try:
                import json
                yield json.loads(payload)
            except json.JSONDecodeError:
                # 非 JSON 行跳过 (e.g. event: 标记)
                continue

    # ── Tools ──

    def invoke_tool(self, name: str, arguments: dict) -> dict:
        resp = self._session.post(
            f"{self.base_url}/v1/tools/{name}/invoke",
            json={"arguments": arguments},
            headers=self._headers(),
            timeout=self.timeout,
        )
        if resp.status_code != 200:
            raise ApiError(resp.status_code, resp.text, f"/v1/tools/{name}/invoke")
        return resp.json()

    # ── Health ──

    def health(self) -> dict:
        resp = self._session.get(
            f"{self.base_url}/v1/system/health",
            timeout=self.timeout,
        )
        if resp.status_code != 200:
            raise ApiError(resp.status_code, resp.text, "/v1/system/health")
        return resp.json()

    # ── Lifecycle ──

    def close(self) -> None:
        self._session.close()