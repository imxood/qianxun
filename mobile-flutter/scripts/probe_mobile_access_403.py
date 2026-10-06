"""查清 mobile-access 返回 403 的原因 —— 这是配对流程该做的事，不是缺陷。

现象
----
`https://192.168.20.2:3443/api/session/list` 连上了（不再是连接失败），
但返回 **403 Forbidden**，且带 `qx_token` cookie。

两个可能：
1. mobile-access 用**自己的**认证（不是 DSH 网关的 `qx_token`），
   那是 dsh-mobile 插件自签的 CA + 配对设备凭据，与网关那条路完全独立
2. 路径不对：mobile-access 也许不把 `/api/*` 透传给 DSH，而是有自己的前缀

**必须分清**，因为方案 B 的 L3 全押在「mobile-access 同源下 request 能调
RPC」上。若是 2，整个方案的通道设计要改。

所以这里依次试：
- 各种路径前缀（`/api/`、`/`、`/mobile-access/`）
- 各种凭据（网关 qx_token、mobile-access 自己的设备 token）
- 看响应体里的错误码 —— 那会直接告诉我们它要什么
"""
from __future__ import annotations

import json
import ssl
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

QIANXUN = Path(r"C:\Users\maxu\.qianxun")
ACCESS = QIANXUN / "dsh-home" / "mobile-access"
ORIGIN = "https://192.168.20.2:3443"
CA = ACCESS / "tls" / "ca.pem"

# mobile-access 自己可能有的端点（从 dsh-mobile 源码/README 抄来的）
PATHS = [
    "/api/session/list",
    "/api/remote.mux",
    "/",
    "/qx-gate",
    "/mobile-access/pair",
    "/mobile-access/custom.js",
    "/mobile-access/custom.css",
    "/health",
]


def probe(path: str, token: str | None) -> tuple[int, str]:
    url = f"{ORIGIN}{path}"
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Cookie"] = f"qx_token={token}"
    data = None
    if path == "/api/session/list":
        data = json.dumps({
            "type": "client-request", "rpcId": "p", "method": "session/list",
            "payload": {"args": {"_request": {}}},
        }).encode()
    # ⚠️ 这里**必须**关证书校验，且不能用 CA 文件。
    #
    # Python 的 `create_default_context(cafile=…)` 走 OpenSSL 的 X509 验证链，
    # 它**要求 CA 证书带 Authority Key Identifier（AKI）**。而 dsh-mobile
    # 自签的 CA 没带 → 报 `Missing Authority Key Identifier`。
    #
    # 这**不是证书无效**：Android / 浏览器 / Node 都不强制要求 AKI，
    # 它们按「精确 Origin 固定」或「用户显式信任」处理。
    # 壳层实现时要用 `PinnedTls.kt` 那种**精确主机固定**，
    # 而不是「把 CA 装进信任库」—— 理由见 08-l3-contract.md。
    #
    # 顺带：这也是 dsh-mobile README `:25` 写的那句
    # 「WebView 仅在证书由该固定 CA 为精确 Origin 和有效期签发时接受
    # 原本不受信任的叶证书」—— 固定 CA 本身有效，AKI 缺失只影响
    # 某些严格实现。
    ctx = ssl.create_default_context()
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE
    req = urllib.request.Request(url, data=data, method="POST" if data else "GET", headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=12, context=ctx) as resp:
            return resp.status, resp.read(400).decode("utf-8", "ignore")
    except urllib.error.HTTPError as exc:
        return exc.code, exc.read(400).decode("utf-8", "ignore")
    except (urllib.error.URLError, OSError) as exc:
        return 0, f"{type(exc).__name__}: {exc}"


def main() -> int:
    settings = json.loads((QIANXUN / "settings.json").read_text(encoding="utf-8"))
    device = next(d for d in settings["remote"]["devices"] if not d.get("revoked"))
    gw_token = device["token"]

    print(f"origin  : {ORIGIN}")
    print(f"CA      : {CA} exists={CA.is_file()}")
    print(f"网关 token: {gw_token[:12]}… ({len(gw_token)} 位)")

    devices_file = ACCESS / "devices.json"
    print(f"\ndevices.json exists={devices_file.is_file()}")
    if devices_file.is_file():
        raw = devices_file.read_text(encoding="utf-8").strip()
        print(f"  内容长度 {len(raw)}  预览 {raw[:220]!r}")

    print(f"\n{'路径':<32} {'无凭据':<14} {'带网关 token':<14}")
    print("-" * 62)
    for path in PATHS:
        a, body_a = probe(path, None)
        b, body_b = probe(path, gw_token)
        print(f"{path:<32} {a:<14} {b:<14}")

    print("\n=== 403 响应体（第 1 个 api 路径，带 token）===")
    _, body = probe("/api/session/list", gw_token)
    print(body[:600])

    print("\n=== 无凭据访问根路径 ===")
    _, body = probe("/", None)
    print(body[:400])

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
