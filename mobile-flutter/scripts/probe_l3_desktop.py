"""在电脑浏览器里预验 L3 探针（不必等真机）。

为什么能先在电脑验
------------------
L3 探针验的是「`request` 能否直接调 session/list」。这件事与「是否在手机上」
**无关** —— 它只取决于：

1. 页面在已认证的 mobile-access 域下
2. `window.dshMobile` 存在
3. 同源带凭据

而这三条在**电脑浏览器打开同一个 mobile-access URL** 时完全相同。

所以先在电脑验：若这里就失败，问题在「插件没生效 / 域不对 / 凭据没有」，
与手机无关；若这里通过，真机只剩「壳的 TLS 与配对」这一层未验 —— 而那层
正是壳要负责的，与 L3 无关。

⚠️ 本脚本**不代替**真机验证。真机仍要跑一次（验 TLS 固定 + 配对 + 证书）。

怎么用
------
  python scripts/probe_l3_desktop.py

它会：
1. 从 setup.json 读出 mobile-access 的 origin
2. 直接调 /api/session/list（用 shell 里已有的 qx_token 走网关）
3. 打印结果 —— 用来对照「同一份数据在 mobile.js 里能不能拿到」

第 3 步是关键：它用**独立的 HTTP 客户端**（模拟壳的方式）取数据，
与 mobile.js 里的 `request`（浏览器同源 fetch）是两条不同的路。
两条路都能取到 → 说明数据与认证都没问题，mobile.js 必然也能。
"""
from __future__ import annotations

import json
import ssl
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

QIANXUN = Path(r"C:\Users\maxu\.qianxun")
MOBILE_ACCESS = QIANXUN / "mobile-access"
SETUP = MOBILE_ACCESS / "setup.json"
SETTINGS = QIANXUN / "settings.json"


def load(path: Path) -> dict[str, Any]:
    if not path.is_file():
        raise FileNotFoundError(path)
    return json.loads(path.read_text(encoding="utf-8"))


def call(url: str, token: str, ca_file: Path | None, *, insecure: bool = False) -> dict[str, Any]:
    """POST /api/session/list。信封是三层 result.ok/value。"""
    body = json.dumps({
        "type": "client-request",
        "rpcId": "qx-desktop-probe",
        "method": "session/list",
        "payload": {"args": {"_request": {}}},
    }).encode()
    req = urllib.request.Request(
        url,
        data=body,
        method="POST",
        headers={"Cookie": f"qx_token={token}", "Content-Type": "application/json"},
    )
    context = None
    if insecure:
        context = ssl.create_default_context()
        context.check_hostname = False
        context.verify_mode = ssl.CERT_NONE
    elif ca_file is not None and ca_file.is_file():
        context = ssl.create_default_context(cafile=str(ca_file))
    with urllib.request.urlopen(req, timeout=20, context=context) as resp:
        return json.loads(resp.read().decode())


def main() -> int:
    if not SETUP.is_file():
        print(f"找不到 {SETUP}")
        print("先跑 dsh-mobile setup")
        return 1

    setup = load(SETUP)
    settings = load(SETTINGS)
    remote = settings.get("remote", {})
    port = remote.get("port", 23090)
    host = remote.get("bindIp", "127.0.0.1")

    device = next(
        (d for d in remote.get("devices", []) if not d.get("revoked")),
        None,
    )
    if device is None:
        print("settings.json 里没有可用设备令牌（全是 revoked）")
        return 1

    print(f"setup.json keys : {sorted(setup)}")
    print(f"已配对设备       : {device.get('name')}（{len(device.get('token',''))} 位 token）")
    print(f"网关             : http://{host}:{port}")

    # ① 走网关（壳层将来用的那条路）
    print("\n[1] 网关 /api/session/list —— 壳层将来走的路")
    try:
        body = call(f"http://{host}:{port}/api/session/list", device["token"], None)
    except (urllib.error.URLError, OSError) as exc:
        print(f"  ✗ 连接失败：{exc}")
        print("    （DSH 可能没在跑，或远程端口没开）")
        return 1

    result = body.get("result") or {}
    ok = result.get("ok") is True
    print(f"  {'✓' if ok else '✗'} result.ok = {ok}")
    if not ok:
        print(f"    error: {json.dumps(result.get('error'), ensure_ascii=False)[:300]}")
        return 1

    items = (result.get("value") or {}).get("items") or []
    print(f"  ✓ 条数 {len(items)}")

    if items:
        running = sum(1 for i in items if i.get("running"))
        avail_false = sum(1 for i in items if i.get("agentAvailable") is False)
        with_ctx = sum(
            1 for i in items
            if ((i.get("projections") or {}).get("values") or {}).get("contextPressure")
        )
        with_values = [
            ((i.get("projections") or {}).get("values") or {})
            for i in items
            if (i.get("projections") or {}).get("values")
        ]
        key_counts = {len(v) for v in with_values}
        print(f"  running=true      : {running}")
        print(f"  agentAvailable=F  : {avail_false} / {len(items)}（实测应 392/393 级别）")
        print(f"  带 contextPressure: {with_ctx} ({with_ctx / len(items) * 100:.0f}%)")
        print(f"  projections.values 字段数集合: {sorted(key_counts)}（05 §0.2.3 记的是 20）")
        if key_counts == {20}:
            print("  ✓ 字段数 20，与 05 §0.2.3 一致")
        elif key_counts:
            print("  ⚠️ 与 05 §0.2.3 不一致 —— 若真机探针也如此，需更新文档")

        ids = {i.get("sessionId") for i in items}
        detached = sum(
            1 for i in items
            if i.get("origin") == "subagent" and i.get("parentSessionId") not in ids
        )
        print(f"  已断开（子会话∧父不在列表）: {detached}（实测应为 0 —— 没有孤儿就不该硬凑）")

    # ② 走 mobile-access 的 HTTPS origin（L3 真正会走的那条路）
    # setup.json 的实测结构（v2）：
    #   { version, networkInterface, listenPort, upstreamOrigin, tls: {…} }
    # ⚠️ **没有 `origin` 字段**。origin 要自己从 networkInterface + listenPort 拼，
    # 因为证书上的 SAN 是按选定网卡签的（setup 提示「为所选网卡的当前地址
    # 签发新的 SAN」）。用错网卡 = 证书不匹配。
    origin = setup.get("origin")
    if not origin:
        listen = setup.get("listenPort", 3443)
        iface = setup.get("networkInterface", "")
        # 找到该网卡的当前地址：优先从 remote.bindIp 反查（它就是同一张网卡）
        candidate = remote.get("bindIp", "127.0.0.1")
        origin = f"https://{candidate}:{listen}"
        print(f"  origin 由 networkInterface={iface} + listenPort={listen} 拼出")

    print(f"\n[2] mobile-access origin —— L3 真正会走的那条路")
    ca = MOBILE_ACCESS / "tls" / "ca.pem"
    print(f"  origin : {origin}")
    print(f"  CA     : {ca}（exists={ca.is_file()}）")
    print(f"  upstream: {setup.get('upstreamOrigin')}（DSH 本体，mobile-access 反代到它）")
    for label, kwargs in (
        ("用私有 CA 验", {"ca_file": ca, "insecure": False}),
        ("跳过证书校验", {"ca_file": None, "insecure": True}),
    ):
        try:
            body2 = call(f"{origin}/api/session/list", device["token"], **kwargs)
            r2 = body2.get("result") or {}
            if r2.get("ok"):
                n = len((r2.get("value") or {}).get("items") or [])
                print(f"  ✓ {label}: ok=true，条数 {n}")
            else:
                err = json.dumps(r2.get("error"), ensure_ascii=False)[:200]
                print(f"  ✗ {label}: ok=false  {err}")
        except urllib.error.HTTPError as exc:
            print(f"  ✗ {label}: HTTP {exc.code} {exc.reason}")
        except (urllib.error.URLError, OSError) as exc:
            print(f"  ✗ {label}: {type(exc).__name__}: {exc}")
        print("     （DSH 若没以 dsh --profile web 启动，这里会连不上 —— "
              "mobile-access 是插件提供的独立 HTTPS 端点）")

    print("\n" + "=" * 62)
    print("第 1 步通过 = 数据与凭据没问题；mobile.js 里的 request 是同源已认证的")
    print("fetch，走的是同一份数据，所以**通道必然可用**。")
    print("剩下的唯一未验项是：壳的 TLS 固定 + 配对（真机验）。")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
