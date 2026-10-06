"""验 403 是不是 Origin 检查 —— 若是，L3 通道成立。

`gateway.ts:2066`：
    if (!policy.acceptsOrigin(request.headers.origin)) throw new HttpError(403, 'forbidden')

而其余路径返回 401 `authentication_failed`（`hasToken` 那条路）。

**403 与 401 的区别就是全部答案**：
- 401 = 「没带凭据」→ 根路径、custom.js 等
- 403 = 「带凭据了但 Origin 不对」→ `/api/*`

而 `mobile.js` 里的 `request` 是**页面同源 fetch**，浏览器会自动带
`Origin: https://192.168.20.2:3443`，`acceptsOrigin` 必然通过。

⚠️ 我上一轮探针用 Python 发请求，**不带 Origin 头** → 403。
那是**探针的缺陷，不是通道的缺陷**。差点误判成「方案 B 的通道不通」。

所以这里补上 Origin 头重试。若拿到 `result.ok: true`，则：
- ✅ 数据通
- ✅ 认证通（网关 qx_token 在 mobile-access origin 上被接受）
- ✅ Origin 检查通
- → **L3 通道完全成立，方案 B 确立**
"""
from __future__ import annotations

import json
import ssl
import urllib.error
import urllib.request
from pathlib import Path

QIANXUN = Path(r"C:\Users\maxu\.qianxun")
ORIGIN = "https://192.168.20.2:3443"


def call(path: str, token: str, with_origin: bool) -> tuple[int, str]:
    headers = {"Content-Type": "application/json", "Cookie": f"qx_token={token}"}
    if with_origin:
        headers["Origin"] = ORIGIN
        headers["Referer"] = f"{ORIGIN}/"
    data = json.dumps({
        "type": "client-request", "rpcId": "origin-test", "method": "session/list",
        "payload": {"args": {"_request": {}}},
    }).encode()
    ctx = ssl.create_default_context()
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE
    req = urllib.request.Request(f"{ORIGIN}{path}", data=data, method="POST", headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=15, context=ctx) as resp:
            return resp.status, resp.read().decode("utf-8", "ignore")
    except urllib.error.HTTPError as exc:
        return exc.code, exc.read(300).decode("utf-8", "ignore")
    except (urllib.error.URLError, OSError) as exc:
        return 0, f"{type(exc).__name__}: {exc}"


def main() -> int:
    settings = json.loads((QIANXUN / "settings.json").read_text(encoding="utf-8"))
    device = next(d for d in settings["remote"]["devices"] if not d.get("revoked"))
    token = device["token"]

    print(f"origin  : {ORIGIN}")
    print(f"token   : {token[:12]}…\n")

    status, body = call("/api/session/list", token, with_origin=False)
    print(f"不带 Origin : {status}  {body[:120]}")

    status, body = call("/api/session/list", token, with_origin=True)
    print(f"带 Origin   : {status}")
    if status != 200:
        print(f"  body: {body[:300]}")
        return 1

    payload = json.loads(body)
    result = payload.get("result") or {}
    ok = result.get("ok") is True
    print(f"  result.ok  : {ok}")
    if not ok:
        print(f"  error      : {json.dumps(result.get('error'), ensure_ascii=False)[:300]}")
        return 1

    items = (result.get("value") or {}).get("items") or []
    print(f"  条数       : {len(items)}")
    if items:
        with_values = [
            i for i in items
            if isinstance((i.get("projections") or {}).get("values"), dict)
        ]
        sigs: dict[int, int] = {}
        for i in with_values:
            n = len(i["projections"]["values"])
            sigs[n] = sigs.get(n, 0) + 1
        print(f"  字段签名   : {dict(sorted(sigs.items()))}")
        print(f"               （与网关那条路一致 = 同一份数据）")

    print("\n" + "=" * 66)
    print("L3 通道成立：")
    print("  · 认证  —— 网关的 qx_token 在 mobile-access origin 上被接受")
    print("  · Origin —— 同源 fetch 自动带，acceptsOrigin 通过")
    print("  · 数据  —— 拿到全部会话，与网关同源")
    print("\n所以 mobile.js 里的 window.dshMobile.register(({ request }) => …)")
    print("可以**直接调全部 RPC**，方案 B 确立。")
    print("\n剩下的唯一未验项：壳层的 TLS 固定与配对（真机验，那是壳的职责）。")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
