"""验千寻自有网关的 L3 通道（不依赖手机）。

为什么能在电脑上验
------------------
千寻的定制层是**服务端注入 + 同源 fetch**：
  · 页面里由网关注入 `qx-mobile-bootstrap.js`（定义 window.qxMobile）
  · 脚本用 `window.fetch('/api/session/list')`，同源自动带 cookie

所以只要在**电脑浏览器**打开千寻网关的 DSH 页面，同源关系与手机上完全
一致。验完这一条，手机上就只剩「WebView 能加载 + 窄屏」两件事。

验四步（与 custom.js 里的探针互为对照）：
  1. /qx-mobile/version —— 定制层是否活（hash 是否反映 custom.js）
  2. /qx-mobile/custom.js —— 我们的探针是否被服务出去
  3. 页面里 qxMobile 是否已注入（读 HTML 找 script 标签）
  4. 带 cookie 调 /api/session/list —— 数据通道

若第 4 步通过 → L3 通道**实测成立**，且用的是千寻自己的远程 URL。
"""
from __future__ import annotations

import json
import re
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

QIANXUN = Path(r"C:\Users\maxu\.qianxun")
GATEWAY = "http://192.168.20.2:23090"


def token() -> str:
    settings = json.loads((QIANXUN / "settings.json").read_text(encoding="utf-8"))
    return next(d for d in settings["remote"]["devices"] if not d.get("revoked"))["token"]


def get(path: str, tok: str | None = None) -> tuple[int, str, dict]:
    headers = {"Cookie": f"qx_token={tok}"} if tok else {}
    req = urllib.request.Request(f"{GATEWAY}{path}", headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=15) as r:
            return r.status, r.read().decode("utf-8", "ignore"), dict(r.headers)
    except urllib.error.HTTPError as e:
        return e.code, e.read(400).decode("utf-8", "ignore"), dict(e.headers)
    except (urllib.error.URLError, OSError) as e:
        return 0, f"{type(e).__name__}: {e}", {}


def post_rpc(tok: str) -> tuple[int, str]:
    body = json.dumps({
        "type": "client-request", "rpcId": "qx-own", "method": "session/list",
        "payload": {"args": {"_request": {}}},
    }).encode()
    req = urllib.request.Request(
        f"{GATEWAY}/api/session/list", data=body, method="POST",
        headers={"Cookie": f"qx_token={tok}", "Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            return r.status, r.read().decode("utf-8", "ignore")
    except urllib.error.HTTPError as e:
        return e.code, e.read(400).decode("utf-8", "ignore")
    except (urllib.error.URLError, OSError) as e:
        return 0, f"{type(e).__name__}: {e}"


def main() -> int:
    tok = token()
    print(f"网关 {GATEWAY}\ntoken {tok[:12]}…\n")

    # ── 1. version 端点 ─────────────────────────────────────────
    print("[1] GET /qx-mobile/version —— 定制层是否活")
    status, body, headers = get("/qx-mobile/version", tok)
    print(f"  {status}  {body[:160]}")
    if status == 200:
        try:
            v = json.loads(body)
            print(f"  ✓ v={v.get('v')}  css={str(v.get('css'))[:16]}  js={str(v.get('js'))[:16]}")
            print(f"    js hash 非空 = {bool(v.get('js'))}（说明 custom.js 已被读到）")
        except json.JSONDecodeError:
            print("  ⚠️ 不是 JSON")
    else:
        print("  ✗ 定制层不可用 —— 网关版本可能不含 mobile_ui 模块")

    # ── 2. custom.js 是否被服务出去 ────────────────────────────
    print("\n[2] GET /qx-mobile/custom.js —— 探针是否在服务")
    status, body, headers = get("/qx-mobile/custom.js", tok)
    print(f"  {status}  长度 {len(body)}")
    print(f"  Content-Type: {headers.get('Content-Type', headers.get('content-type', '—'))}")
    print(f"  ETag: {headers.get('ETag', headers.get('etag', '—'))}")
    is_fallback = headers.get("x-qx-mobile-custom", headers.get("X-Qx-Mobile-Custom", "")) == "fallback"
    print(f"  是占位回退: {is_fallback}")
    if status == 200 and "qxMobile.register" in body:
        print("  ✓ 探针内容已在服务（含 qxMobile.register）")
    elif status == 200:
        print(f"  内容预览: {body[:160]!r}")

    # ── 3. 页面里 bootstrap 有没有被注入 ───────────────────────
    print("\n[3] 页面里 qx-mobile/bootstrap.js 是否被注入")
    status, html, _ = get("/", tok)
    print(f"  根页面 {status}  长度 {len(html)}")
    scripts = re.findall(r'<script[^>]*src="([^"]+)"', html)
    for s in scripts:
        mark = " ← 定制层引导" if "qx-mobile" in s else ""
        print(f"    script: {s}{mark}")
    print(f"  提到 qxMobile: {'qxMobile' in html}")

    # ── 4. 数据通道 ─────────────────────────────────────────────
    print("\n[4] 带 cookie 调 /api/session/list")
    status, body = post_rpc(tok)
    print(f"  {status}")
    if status != 200:
        print(f"  {body[:200]}")
        return 1
    payload = json.loads(body)
    r = payload.get("result") or {}
    if r.get("ok") is not True:
        print(f"  ✗ ok=false  {json.dumps(r.get('error'), ensure_ascii=False)[:200]}")
        return 1
    items = (r.get("value") or {}).get("items") or []
    print(f"  ✓ ok=true  条数 {len(items)}")
    sigs: dict[int, int] = {}
    for i in items:
        v = (i.get("projections") or {}).get("values")
        if isinstance(v, dict):
            sigs[len(v)] = sigs.get(len(v), 0) + 1
    print(f"  ✓ 字段签名 {dict(sorted(sigs.items()))}")

    print("\n" + "=" * 70)
    print("判定")
    if status == 200 and items:
        print("✅ **千寻自有网关的 L3 通道实测成立**，且用的是：")
        print("   · URL   http://192.168.20.2:23090（千寻自己的远程网关）")
        print("   · 认证   现有 qx_token，无需新配对流程")
        print("   · 入口   window.qxMobile.register（千寻注入）")
        print("   · 文件   <dsh_home>/mobile-access/custom.{js,css}，5 秒热生效")
        print("\n   这条路**优于** dsh-mobile 的 3443：")
        print("   · 不需要自签 CA / TLS 固定（明文 HTTP）")
        print("   · 不需要四层认证（Origin/Session/CSRF/CIDR）")
        print("   · 基础设施已在千寻仓库，不需要装任何插件")
        print("\n   壳层剩下的全部工程量：")
        print("   · WebView 宿主 + 返回键语义 + 设备列表入口")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
