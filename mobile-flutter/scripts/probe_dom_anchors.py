"""抓 DSH 页面的真实 DOM 骨架，供 custom.css / custom.js 写选择器。

为什么必须抓
------------
`custom.js` 要改 DSH 的界面，就必须知道**稳定的锚点**。
dsh-mobile 自己给出了警告：它用的是 `[data-disclosure-row]`、
`[data-question-key]`、`[class*="_ioSection"]` 这类**语义属性**，
而不是 class 名 —— 说明 DSH 的 class 名是构建期哈希（实测形如
`hHd-Xa_panelRow`、`bhn1Oq_searchButton`），**每次构建都会变**。

所以规则是：**只用 data-* 属性、role、aria-*，不用 class**。
而哪些 data-* 属性真实存在，必须抓，不能猜。

做法：调网关拿 DSH 的 JS bundle，从里面把**稳定字面量**抠出来 ——
比在浏览器里 dump DOM 更可靠（不需要起浏览器，且能覆盖未渲染的分支）。

用法：python scripts/probe_dom_anchors.py
"""
from __future__ import annotations

import json
import re
import urllib.error
import urllib.parse
import urllib.request
from collections import Counter
from pathlib import Path
from typing import Any

QIANXUN = Path(r"C:\Users\maxu\.qianxun")
GATEWAY = "http://192.168.20.2:23090"

# 允许出现在 custom.js 里的锚点前缀。class 名一律排除 ——
# 实测形如 hHd-Xa_ / bhn1Oq_ / YDXeBa_ / pI_x6G_，是构建期哈希。
ALLOW_ATTR = re.compile(r"^(data-[a-z-]+|aria-[a-z]+|role)$")
DATA_ATTR = re.compile(r"[\"'](data-[a-z0-9-]+)[\"']\s*[:=]")
ROLE_LIT = re.compile(r"role[\"']?\s*[:=]\s*[\"']([a-z]+)[\"']")
ARIA_LIT = re.compile(r"[\"'](aria-[a-z]+)[\"']\s*[:=]")


def token() -> str:
    settings = json.loads((QIANXUN / "settings.json").read_text(encoding="utf-8"))
    return next(d for d in settings["remote"]["devices"] if not d.get("revoked"))["token"]


def get(path: str, tok: str) -> tuple[int, bytes]:
    # ⚠️ 路径拼接必须走 urljoin，不能字符串拼。
    # 「plugins/…」这种**没有前导斜杠**的相对路径，直接拼到 origin 后面
    # 会变成 `http://host:23090plugins/…` → urllib 报
    # `nonnumeric port: '23090plugins'`。
    # 这类错误信息完全看不出根因（它报的是端口不是路径），所以写死规则。
    url = urllib.parse.urljoin(f"{GATEWAY}/", path.lstrip("/"))
    req = urllib.request.Request(url, headers={"Cookie": f"qx_token={tok}"})
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read(400)
    except (urllib.error.URLError, OSError) as e:
        return 0, str(e).encode()


def main() -> int:
    tok = token()
    status, html_bytes = get("/", tok)
    if status != 200:
        print(f"根页面 {status}")
        return 1
    html = html_bytes.decode("utf-8", "ignore")
    print(f"根页面 {len(html)} 字节\n")

    # ── 收集要分析的 JS 资源 ────────────────────────────────────
    scripts = re.findall(r'<script[^>]*src="([^"]+)"', html)
    print(f"发现 {len(scripts)} 个 script：")
    for s in scripts:
        print(f"  {s}")

    # 只取同源相对路径（plugins/… 那条带 HTML 转义的 &amp;）
    targets: list[str] = []
    for s in scripts:
        clean = s.replace("&amp;", "&")
        if clean.startswith("http"):
            continue
        if not (clean.endswith(".js") or ".js?" in clean or ".js&" in clean):
            continue
        targets.append(clean)
    print(f"\n抓取 {len(targets)} 个 JS…")

    all_data: Counter[str] = Counter()
    all_aria: Counter[str] = Counter()
    all_role: Counter[str] = Counter()
    class_prefixes: Counter[str] = Counter()
    total = 0

    for path in targets:
        st, body = get(path, tok)
        if st != 200:
            print(f"  ✗ {path} → {st}")
            continue
        text = body.decode("utf-8", "ignore")
        total += len(text)
        all_data.update(DATA_ATTR.findall(text))
        all_aria.update(ARIA_LIT.findall(text))
        all_role.update(ROLE_LIT.findall(text))
        # class 名形态：xxx-Xxx_ 前缀（构建期哈希）
        for m in re.findall(r"[\"']([a-z]{2,4}-[A-Za-z]{3}-[a-zA-Z0-9]{2,6}_[a-zA-Z]+)", text):
            class_prefixes[m.rsplit("_", 1)[0]] += 1
        print(f"  ✓ {path[:64]}  {len(text)} 字节")

    print(f"\n合计 {total} 字节 JS\n")

    # ── 输出：可用的稳定锚点 ────────────────────────────────────
    print("=" * 70)
    print("✅ data-* 属性（custom.js 的安全锚点）")
    for name, count in all_data.most_common(60):
        print(f"  {name:<34} {count:>5}")

    print("\n" + "=" * 70)
    print("✅ aria-* 属性")
    for name, count in all_aria.most_common(30):
        print(f"  {name:<34} {count:>5}")

    print("\n" + "=" * 70)
    print("✅ role 字面量")
    for name, count in all_role.most_common(20):
        print(f"  {name:<34} {count:>5}")

    print("\n" + "=" * 70)
    print("⛔ class 名形态（**不可用作锚点**，构建期哈希）")
    for name, count in class_prefixes.most_common(12):
        print(f"  {name:<34} {count:>5}")
    if not class_prefixes:
        print("  （未匹配到，可能是压缩后形态不同）")
    print("\n  实测样例：hHd-Xa_panelRow / bhn1Oq_searchButton / YDXeBa_projectRow")
    print("  这些是构建期哈希，DSH 每次构建都会变 —— 写进 custom.js 必然失效。")

    # ── 落盘 ───────────────────────────────────────────────────
    out = Path(
        r"E:\develop\dsh-workspace\qianxun\mobile-flutter"
        r"\docs\probe\dom-anchors.json"
    )
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps({
        "_probe": {
            "note": (
                "DSH 页面的稳定锚点清单（从 JS bundle 抠出的 data-*/aria-*/role 字面量）。"
                "custom.js 只能用这些，**绝不能用 class 名**（构建期哈希）。"
            ),
            "source": "probe_dom_anchors.py",
            "jsBytesScanned": total,
            "scripts": targets,
        },
        "dataAttributes": dict(all_data.most_common()),
        "ariaAttributes": dict(all_aria.most_common()),
        "roles": dict(all_role.most_common()),
        "classPrefixesDoNotUse": dict(class_prefixes.most_common(20)),
    }, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"\n已写入 {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
