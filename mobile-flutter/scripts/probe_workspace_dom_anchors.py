"""在电脑上用宽视口读 DSH 的工作区 DOM 结构（不受 Preview Notice 干扰）。

为什么换这条路
--------------
真机上每次热替换 custom.js 页面就重载，DSH 的「Preview Notice」模态框
就再弹一次，而它**吞掉所有点击** —— 于是「展开工作区侧栏」点不动，
截图永远停在弹窗上，验证无法进行。
（实测该标记不在 settings/storages 的文件里，应该在 localStorage。）

而电脑上：预览提示在**已经访问过一次之后**就不会再弹，且
Browser 可以用 DOM 查询直接把结构抠出来。

所以这里换成：`python` 侧拉 bundle + 用正则找工作区渲染的锚点，
再配合 `role="treeitem"` 这类**实测存在的语义属性**确认。

用法：
  python scripts/probe_workspace_dom_anchors.py
"""
from __future__ import annotations

import json
import re
import urllib.error
import urllib.request
from collections import Counter
from pathlib import Path
from typing import Any

QIANXUN = Path(r"C:\Users\maxu\.qianxun")
GATEWAY = "http://192.168.20.2:23090"


def token() -> str:
    settings = json.loads((QIANXUN / "settings.json").read_text(encoding="utf-8"))
    return next(
        d for d in settings["remote"]["devices"] if not d.get("revoked")
    )["token"]


def get(path: str) -> tuple[int, str]:
    req = urllib.request.Request(
        f"{GATEWAY}{path}", headers={"Cookie": f"qx_token={token()}"}
    )
    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            return resp.status, resp.read().decode("utf-8", "ignore")
    except urllib.error.HTTPError as exc:
        return exc.code, exc.read(300).decode("utf-8", "ignore")
    except (urllib.error.URLError, OSError) as exc:
        return 0, f"{type(exc).__name__}: {exc}"


def main() -> int:
    # ① 根页面里有哪些 chunk
    status, html = get("/")
    if status != 200:
        print(f"根页面 {status}")
        return 1
    scripts = re.findall(r'<script[^>]*src="([^"]+)"', html)
    links = re.findall(r'<link[^>]*href="([^"]+\.js[^"]*)"', html)
    print(f"根页面 {len(html)} 字节")
    print(f"script {len(scripts)} 个:")
    for s in scripts:
        print(f"  {s}")
    for l in links:
        print(f"  (link) {l}")

    # ② 主 bundle
    status, js = get("/assets/index-5SrrfWpU.js")
    print(f"\n主 bundle {status}  {len(js)} 字节")
    if status != 200:
        return 1

    # ③ 找工作区相关的 i18n 键与 DOM 属性
    # 说明：压缩后变量名会变形，但**字符串字面量**会保留。
    # 我们要找的是形如 "workspace"/"ungrouped"/"sessionIds" 的字面量。
    literals = Counter(re.findall(r'"([A-Za-z][A-Za-z0-9_\-]{3,30})"', js))
    interesting = [
        (k, v) for k, v in literals.items()
        if any(h in k.lower() for h in (
            "workspace", "ungroup", "group", "pinned", "archived", "treeitem",
            "project", "session", "treecount", "expand",
        ))
    ]
    interesting.sort(key=lambda kv: -kv[1])
    print("\n=== bundle 里工作区相关的字符串字面量（前 40）===")
    for k, v in interesting[:40]:
        print(f"  {k:<34} {v}")

    # ④ 确认语义属性是否真在 bundle 里（这是我们能安全依赖的锚点）
    print("\n=== 可作锚点的语义属性（在 bundle 里出现的）===")
    for attr in (
        "role", "aria-level", "aria-expanded", "aria-selected", "aria-label",
        "data-disclosure-row", "data-expanded", "data-side", "data-portal",
        "data-pinned", "data-phase", "data-running", "data-dockkit-column",
    ):
        n = js.count(f'"{attr}"') + js.count(f"'{attr}'")
        if n:
            print(f"  {attr:<26} {n} 次")

    # ⑤ 明确回答：有没有 workspace RPC
    print("\n=== 有没有 workspace 相关的 RPC 端点 ===")
    for m in re.findall(r'"(workspace[sA-Za-z]*)/([a-zA-Z]+)"', js):
        print(f"  {m[0]}/{m[1]}")
    if not any("workspace" in s for s, _ in [(k, v) for k, v in interesting]):
        print("  （bundle 里没有 workspace 相关的端点字面量）")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
