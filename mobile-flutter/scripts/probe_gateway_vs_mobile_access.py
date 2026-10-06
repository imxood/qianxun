"""验一件事：**能不能用千寻自己的远程网关，而不是 dsh-mobile 的 3443？**

为什么问这个
------------
方案 B 我原本假设「必须走 mobile-access 的 3443 端点」，
因为 `dsh-mobile/README.zh-CN.md:5` 说 App 访问的是插件提供的那个地址。

但千寻**自己已经有一个远程网关**：`192.168.20.2:23090`，
我们此前配对用的就是它（`settings.json` 的 `remote` 段，
设备 `maxu_android`，token 64 位）。而且实测它能取到全部 394 条会话。

若那条路也能用 `mobile.js`，就省掉：
  · 自签 CA 与 TLS 固定（明文 HTTP，没证书问题）
  · mobile-access 的四层认证（Origin/Session/CSRF/CIDR）
  · 一整套配对流程（native-pair、renew、设备管理）
壳层只剩「加载一个 WebView + 设置 cookie」。

**但这条路有三个必须验的点，一个都不能猜：**

1. **网关会不会注入 `window.dshMobile`？** 那是 dsh-mobile 插件注入的
   （`client.ts:84-89`）。没有它，`mobile.js` 就没有挂载点。
   —— 这是**致命的一条**，插件没装在网关那条路上。

2. **网关上的 DSH 是同一个实例吗？** 若是同一个，`mobile-access`
   装的插件应该对它可见…但插件是 per-profile 的，而网关可能是
   另一个 profile 起的服务。

3. **WebView 加载 `http://192.168.20.2:23090/` 能不能进 DSH 界面？**
   需不需要额外的 `?token=`？

所以这里逐条实测。结论会直接决定壳层要不要写 TLS 固定。
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
MOBILE_ACCESS = "https://192.168.20.2:3443"


def token() -> str:
    settings = json.loads((QIANXUN / "settings.json").read_text(encoding="utf-8"))
    remote = settings["remote"]
    return next(d for d in remote["devices"] if not d.get("revoked"))["token"]


def fetch(url: str, tok: str | None = None, extra: dict[str, str] | None = None) -> tuple[int, str, dict]:
    headers = {"Cookie": f"qx_token={tok}"} if tok else {}
    if extra:
        headers.update(extra)
    req = urllib.request.Request(url, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            return resp.status, resp.read().decode("utf-8", "ignore"), dict(resp.headers)
    except urllib.error.HTTPError as exc:
        return exc.code, exc.read(600).decode("utf-8", "ignore"), dict(exc.headers)
    except (urllib.error.URLError, OSError) as exc:
        return 0, f"{type(exc).__name__}: {exc}", {}


def main() -> int:
    tok = token()
    print(f"网关     : {GATEWAY}")
    print(f"mobile-access: {MOBILE_ACCESS}")
    print(f"token    : {tok[:12]}…\n")

    # ── 1. 网关根页面是不是 DSH 界面？ ─────────────────────────
    print("[1] 网关根页面（不带 token）")
    status, body, headers = fetch(f"{GATEWAY}/")
    print(f"  状态 {status}  长度 {len(body)}")
    print(f"  Content-Type: {headers.get('Content-Type', headers.get('content-type', '—'))}")
    if status == 200:
        is_html = "<!doctype html" in body.lower() or "<html" in body.lower()
        print(f"  是 HTML 页面: {is_html}")
        title = re.search(r"<title[^>]*>(.*?)</title>", body, re.I | re.S)
        if title:
            print(f"  title: {title.group(1).strip()[:80]}")
        # 看有没有内联 dshMobile 注入
        print(f"  提到 dshMobile: {'dshMobile' in body}")
        print(f"  提到 __DSH_MOBILE_FRONTEND__: {'__DSH_MOBILE_FRONTEND__' in body}")
        print(f"  首 200 字: {body[:200]!r}")

    # ── 2. 带 token 的根页面 ────────────────────────────────────
    print("\n[2] 网关根页面（带 qx_token）")
    status, body, headers = fetch(f"{GATEWAY}/", tok)
    print(f"  状态 {status}  长度 {len(body)}")
    if status == 200:
        print(f"  提到 dshMobile: {'dshMobile' in body}")
        for m in re.finditer(r'<script[^>]*src="([^"]+)"', body):
            print(f"    script: {m.group(1)}")
    else:
        print(f"  body 预览: {body[:250]}")

    # ── 3. 网关有没有 mobile-access 相关的路由 ──────────────────
    print("\n[3] 网关上的 mobile-access 路由")
    for path in (
        "/mobile-access/custom.js",
        "/mobile-access/custom.css",
        "/api/mobile-access/devices",
        "/qx-gate",
    ):
        s, b, _ = fetch(f"{GATEWAY}{path}", tok)
        print(f"  {path:<34} {s}  {b[:80]!r}")

    # ── 4. mobile-access 那边确认 dshMobile 是插件注入的 ────────
    print("\n[4] 对照：mobile-access 侧（插件生效的地方）")
    import ssl
    ctx = ssl.create_default_context()
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE
    try:
        req = urllib.request.Request(
            f"{MOBILE_ACCESS}/mobile-access/custom.js",
            headers={"Origin": MOBILE_ACCESS, "Referer": f"{MOBILE_ACCESS}/"},
        )
        with urllib.request.urlopen(req, timeout=12, context=ctx) as resp:
            js = resp.read(400).decode("utf-8", "ignore")
        print(f"  custom.js 状态 200")
        print(f"  提到 dshMobile: {'dshMobile' in js}")
        print(f"  预览: {js[:200]!r}")
    except urllib.error.HTTPError as exc:
        print(f"  custom.js → {exc.code}（未配对时 401 是正常的）")
    except (urllib.error.URLError, OSError) as exc:
        print(f"  custom.js → {type(exc).__name__}: {exc}")

    print("\n" + "=" * 70)
    print("判读要点：")
    print("  · 若网关根页面**没有** dshMobile 注入 → 那条路**不能**跑 L3，")
    print("    因为 mobile.js 没有挂载点。方案 B 仍必须走 3443。")
    print("  · 若有注入 → 可以省掉 TLS 固定与四层认证，壳层大幅简化。")
    print("  · dshMobile 是 dsh-mobile 插件注入的（client.ts:84-89），")
    print("    而插件装在 --profile web；网关 23090 是哪个 profile 起的，")
    print("    决定了它能不能看见这个插件。")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
