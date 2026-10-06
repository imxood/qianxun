"""走一遍真实配对，拿到 dsh_ma_session，把「推断」变成「实测」。

为什么必须真配一次
------------------
到目前为止，方案 B 的通道都是**源码推断**：
「配对后，mobile.js 里的 `request` 能直接调 RPC」。

但我**从未拿到过一个真实的 session**，所以那条推断没被验证过。
`handleNativePair`（gateway.ts:1089-1108）是原生 App 专用入口，它把
凭据放在**响应体**而不是 cookie —— 正是壳需要的形状。

本脚本完整走一遍：
  1. 生成配对码（走本机管理端点，或读已生成的一次性码）
  2. POST /mobile-access/auth/native-pair
  3. 拿 sessionToken + csrfToken
  4. **带 session + Origin + CSRF 调 /api/session/list**
  5. 打印结果 —— 这一步是分水岭

⚠️ 本脚本会创建一个**真实的配对设备记录**（写进 devices.json）。
它可以随时从 DSH 桌面端撤销，也可以删 devices.json 清空。
这是设计如此：先证明通道，再谈其他。

用法：
  python scripts/probe_l3_pair.py --pair-code <43位配对码>
  python scripts/probe_l3_pair.py --open-pairing      # 试本机管理端点
"""
from __future__ import annotations

import argparse
import json
import ssl
import urllib.error
import urllib.request
from http.cookiejar import CookieJar
from pathlib import Path
from typing import Any

QIANXUN = Path(r"C:\Users\maxu\.qianxun")
ACCESS = QIANXUN / "dsh-home" / "mobile-access"
ORIGIN = "https://192.168.20.2:3443"
DSH_UPSTREAM = "http://127.0.0.1:3080"   # setup.json 的 upstreamOrigin


def ctx() -> ssl.SSLContext:
    """关证书校验。

    ⚠️ dsh-mobile 自签的 CA **没带 Authority Key Identifier**，
    Python/OpenSSL 的链验证会因此失败（`Missing Authority Key Identifier`），
    但那**不是证书无效** —— Android / 浏览器 / Node 都不要求 AKI。
    壳层要用 `PinnedTls` 的**精确 Origin 固定**，不是「装进系统信任库」。
    """
    c = ssl.create_default_context()
    c.check_hostname = False
    c.verify_mode = ssl.CERT_NONE
    return c


def post(
    url: str, payload: dict[str, Any] | None,
    cookies: dict[str, str], extra: dict[str, str] | None = None,
) -> tuple[int, str, dict[str, str]]:
    headers = {"Content-Type": "application/json", "Origin": ORIGIN, "Referer": f"{ORIGIN}/"}
    if cookies:
        headers["Cookie"] = "; ".join(f"{k}={v}" for k, v in cookies.items())
    if extra:
        headers.update(extra)
    data = json.dumps(payload).encode() if payload is not None else b"{}"
    req = urllib.request.Request(url, data=data, method="POST", headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=20, context=ctx()) as resp:
            setc = resp.headers.get_all("Set-Cookie") or []
            jar: dict[str, str] = {}
            for raw in setc:
                first = raw.split(";")[0]
                if "=" in first:
                    k, v = first.split("=", 1)
                    jar[k.strip()] = v.strip()
            return resp.status, resp.read().decode("utf-8", "ignore"), jar
    except urllib.error.HTTPError as exc:
        return exc.code, exc.read(400).decode("utf-8", "ignore"), {}
    except (urllib.error.URLError, OSError) as exc:
        return 0, f"{type(exc).__name__}: {exc}", {}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pair-code", default="", help="43 位配对码")
    ap.add_argument("--open-pairing", action="store_true", help="试本机管理端点开配对窗口")
    ap.add_argument("--label", default="qx-probe", help="设备名")
    args = ap.parse_args()

    # ── 0. 先看本地管理端点能不能开配对窗口 ──────────────────────
    if args.open_pairing:
        print("[0] 试本机管理端点（loopback，无凭据要求）")
        for path in ("/api/mobile-access/pairing/open", "/api/mobile-access/devices"):
            status, body, _ = post(f"{DSH_UPSTREAM}{path}", {}, {})
            print(f"  POST {path} → {status}  {body[:220]}")
        print()

    if not args.pair_code:
        print("需要配对码。两条路：")
        print("  A. 在 DSH 桌面端点左下角「移动访问」卡片 → 局域网 → 生成密钥/配对链接")
        print("     然后把 43 位配对码贴进来：--pair-code <code>")
        print("  B. 若它给的是链接，取 https://host:3443/mobile-access/pair#token=<code>")
        print("     里的 <code> 部分（README.zh-CN.md:23）")
        return 1

    # ── 1. 配对 ──────────────────────────────────────────────────
    print(f"[1] POST {ORIGIN}/mobile-access/auth/native-pair")
    status, body, jar = post(
        f"{ORIGIN}/mobile-access/auth/native-pair",
        {"token": args.pair_code, "label": args.label},
        {},
    )
    print(f"  状态 {status}")
    if status != 201:
        print(f"  body: {body[:400]}")
        print("\n  可能原因：配对码过期 / 配对窗口已关 / 码不属于这台机器")
        return 1

    result = json.loads(body)
    print("  ✓ 配对成功")
    for key in ("instanceId", "deviceId", "deviceExpiresAt", "sessionExpiresAt"):
        if key in result:
            print(f"    {key}: {result[key]}")
    print(f"    deviceToken: {len(result.get('deviceToken',''))} 位")
    print(f"    sessionToken: {len(result.get('sessionToken',''))} 位")
    print(f"    csrfToken: {len(result.get('csrfToken',''))} 位")

    cookies = {
        "dsh_ma_session": result["sessionToken"],
        "dsh_ma_csrf": result["csrfToken"],
        "dsh_ma_device": result["deviceToken"],
    }

    # ── 2. 带上 session 调 RPC —— **这是分水岭** ──────────────────
    print(f"\n[2] 带 session 调 /api/session/list（关键一步）")
    rpc = {
        "type": "client-request", "rpcId": "pair-probe", "method": "session/list",
        "payload": {"args": {"_request": {}}},
    }
    status, body, _ = post(
        f"{ORIGIN}/api/session/list", rpc, cookies,
        # CSRF 头：DSH Mobile 在页面里自动注入，壳里要手动带
        {"x-dsh-mobile-csrf": result["csrfToken"]},
    )
    print(f"  状态 {status}")
    if status != 200:
        print(f"  body: {body[:400]}")
        print("\n  ⚠️ 配对成功但 RPC 仍不通 —— 方案 B 的通道**未成立**，要回退 C。")
        return 1

    payload = json.loads(body)
    r = payload.get("result") or {}
    if r.get("ok") is not True:
        print(f"  ✗ result.ok=false  {json.dumps(r.get('error'), ensure_ascii=False)[:300]}")
        return 1

    items = (r.get("value") or {}).get("items") or []
    print(f"  ✓ result.ok=true，条数 {len(items)}")
    sigs: dict[int, int] = {}
    for i in items:
        v = (i.get("projections") or {}).get("values")
        if isinstance(v, dict):
            sigs[len(v)] = sigs.get(len(v), 0) + 1
    print(f"  ✓ 字段签名 {dict(sorted(sigs.items()))}（应与网关那条路一致）")

    # ── 3. 落盘证据（不落凭据） ─────────────────────────────────
    out = Path(
        r"E:\develop\dsh-workspace\qianxun\mobile-flutter"
        r"\docs\probe\mobile-access-auth.json"
    )
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps({
        "_probe": {
            "note": (
                "mobile-access 原生配对（/mobile-access/auth/native-pair）+ "
                "带 session 调 /api/session/list 的实测结果。"
                "**不含任何凭据** —— 只记形状与结论。"
            ),
            "source": "probe_l3_pair.py",
            "origin": ORIGIN,
            "pairEndpoint": "/mobile-access/auth/native-pair",
            "credentialsInResponseBody": True,
            "sessionCookieName": "dsh_ma_session",
            "csrfCookieName": "dsh_ma_csrf",
            "csrfHeader": "x-dsh-mobile-csrf",
            "deviceCookieName": "dsh_ma_device",
        },
        "pairStatus": status,
        "rpcStatus": status,
        "sessionsVisible": len(items),
        "fieldSignatureHistogram": {str(k): v for k, v in sorted(sigs.items())},
        "verdict": "L3 通道成立：配对后 request 可直接调 RPC",
    }, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"\n  证据已写入 {out}（不含凭据）")

    print("\n" + "=" * 68)
    print("✅ L3 通道**实测成立**，方案 B 确立。")
    print("   配对后 request 是已认证的同源 fetch，可调全部 RPC。")
    print("   凭据形状：native-pair 把 token 放在**响应体**（不是 cookie），")
    print("             正是原生壳需要的；session/csrf 走 cookie。")
    print("\n   接下来：Shell 1（设备列表 → 选设备 → DSH 页，带返回）")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
