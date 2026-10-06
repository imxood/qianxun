"""找出 DSH Preview Notice 的存储位置并清掉 —— 它会无限挡住真机验证。

为什么必须解决它
----------------
DSH 0.2 每次页面重载都弹「Preview Notice」模态框，而它是**模态**的：
吞掉所有点击。于是：
  · 「展开工作区侧栏」点不动
  · 截图永远停在这个框上
  · 每部署一次 custom.js 就要手点一次 Continue
它是「静默失败」之王 —— 看起来像我们的代码没生效，其实是别人的弹窗。

之前扫过 settings/storages 下的文件，**没有** preview 相关键，
所以它在浏览器侧：localStorage / IndexedDB / Cookie。

路线（从便宜到贵）
------------------
1. 先看 DSH 自己的存储有没有「已看过预览版」这类标记
2. 没有就查 bundle 里的 i18n 键 —— 拿到键名就知道存在哪
3. 仍不确定，就在页面里直接关掉它（把它的容器移除，
   并标记「本会话不再显示」）

第 3 条是**最可靠**的：不管它存在哪，只要在渲染后移除 DOM，
点击就能穿透。但要小心：它是模态，可能挂在 body 上遮住一切。
所以做法是：找到含「Preview Notice」文案的最近祖先容器，
设置 `display:none` 而不是删节点（删了可能触发它的重渲染逻辑）。
"""
from __future__ import annotations

import json
import re
import urllib.error
import urllib.request
from pathlib import Path

QIANXUN = Path(r"C:\Users\maxu\.qianxun")
GATEWAY = "http://192.168.20.2:23090"


def token() -> str:
    settings = json.loads((QIANXUN / "settings.json").read_text(encoding="utf-8"))
    return next(
        d for d in settings["remote"]["devices"] if not d.get("revoked")
    )["token"]


def get(path: str, timeout: int = 90) -> tuple[int, str]:
    req = urllib.request.Request(
        f"{GATEWAY}{path}", headers={"Cookie": f"qx_token={token()}"}
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return resp.status, resp.read().decode("utf-8", "ignore")
    except urllib.error.HTTPError as exc:
        return exc.code, exc.read(200).decode("utf-8", "ignore")
    except (urllib.error.URLError, OSError) as exc:
        return 0, f"{type(exc).__name__}: {exc}"


def main() -> int:
    st, html = get("/")
    print(f"根页面 {st}  {len(html)} 字节")
    if st != 200:
        return 1

    # 1) 两个 plugin 组合都扫一遍
    hrefs = re.findall(r'<link[^>]*href="([^"]+)"', html)
    scripts = re.findall(r'<script[^>]*src="([^"]+)"', html)
    targets = [
        h for h in hrefs
        if h.endswith(".js") or ".js?" in h
    ] + scripts

    print(f"扫 {len(targets)} 个 JS 资源，找 preview 相关的 i18n 键…\n")
    hits: dict[str, list[str]] = {}
    for href in targets:
        path = href.replace("&amp;", "&")
        st2, js = get("/" + path.lstrip("./"))
        if st2 != 200 or len(js) < 500:
            continue
        for key in re.findall(r'"([a-zA-Z][a-zA-Z0-9_.\-]{3,50})"\s*:\s*"[^"]{0,120}"', js):
            low = key.lower()
            if any(h in low for h in ("preview", "notice", "onboard", "firstrun", "first-run", "whatsnew")):
                hits.setdefault(key, []).append(path[:60])
        # 也找值里含 "preview version" 的
        for value in re.findall(r'"([^"]{4,120}[Pp]review[^"]{0,60})"', js):
            for key in re.findall(r'"([a-zA-Z][a-zA-Z0-9_.\-]{3,50})"\s*:\s*"' + re.escape(value) + '"', js):
                hits.setdefault(key, []).append(path[:60])

    if not hits:
        print("❗ bundle 里没有 preview/notice 相关的 i18n 键")
        print("   → 它不是通过 i18n 字典来的，可能直接内联文案或来自别处。")
        print("   → 改用路线 3：在 custom.js 里直接把该模态移除。")
        return 1

    print(f"=== 命中 {len(hits)} 个键 ===")
    for key in sorted(hits):
        print(f"  {key:<40} in {sorted(set(hits[key]))[0]}")

    # 2) 在 DSH 的文件存储里找这些键
    print("\n=== 在 DSH 存储文件里查这些键 ===")
    for path in (QIANXUN / "dsh-home").rglob("*.json"):
        if "node_modules" in str(path) or path.stat().st_size > 2_000_000:
            continue
        try:
            text = path.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        for key in hits:
            if key in text:
                print(f"  {path.relative_to(QIANXUN)}  含 {key}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
