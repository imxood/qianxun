"""从 workspace 插件的 chunk 里抠出三样东西：方法名、i18n 文案、DOM 锚点。

背景
----
根页面的 link 标签里那条 10.6MB 的 plugin 组合 URL 里有
`@deepseek-ai/dsh-client-ui-workspace` 与 `dsh-api-workspace-controller`。
里面能查到这些字面量（实测 10,600,622 字节里全部命中）：

  Ungrouped / Pinned / Archived / no-workspace / noWorkspace
  sessionIds / workspaceIds / pinnedSessionIds / archivedSessionIds
  heroWorkspaceRow / workspaceRow / workspaceLabel / workspaceDropBefore

这说明**「未分组 / 置顶 / 归档」在 DSH 里是真实概念**，且有 i18n 文案。
而这些是我们手机端要正确显示就必须知道的东西 —— 之前我只能靠猜。

本脚本把它们连同**方法名**一起抠出来，作为第 2 条修正的依据。

用法：python scripts/probe_workspace_chunk.py
"""
from __future__ import annotations

import json
import re
import urllib.error
import urllib.request
from pathlib import Path

QIANXUN = Path(r"C:\Users\maxu\.qianxun")
GATEWAY = "http://192.168.20.2:23090"
OUT = Path(
    r"E:\develop\dsh-workspace\qianxun\mobile-flutter"
    r"\docs\probe\workspace-chunk.json"
)


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


def around(js: str, needle: str, radius: int = 160) -> str:
    index = js.find(needle)
    if index == -1:
        return ""
    return js[max(0, index - radius): index + radius].replace("\n", " ")


def main() -> int:
    status, html = get("/")
    if status != 200:
        print(f"根页面 {status}")
        return 1

    hrefs = re.findall(r'<link[^>]*href="([^"]+)"', html)
    big = next(
        (h for h in hrefs if "dsh-client-ui-workspace" in h), None
    )
    if big is None:
        print("根页面里找不到含 dsh-client-ui-workspace 的 plugin 组合")
        return 1

    clean = big.replace("&amp;", "&")
    st, js = get("/" + clean.lstrip("./"))
    print(f"chunk {st}  {len(js)} 字节")

    result: dict = {}

    # ── 1. i18n 文案：找 zh/en 字典里 Ungrouped / Pinned / Archived 的译文 ──
    i18n = {}
    for key in ("Ungrouped", "Pinned", "Archived", "Workspaces", "Workspace"):
        snippet = around(js, f'"{key}"', 200)
        if snippet:
            i18n[key] = snippet
    result["i18nContext"] = i18n
    print("\n=== i18n 上下文 ===")
    for key, snippet in i18n.items():
        print(f"  {key}:")
        print(f"    {snippet[:190]}")

    # ── 2. 方法名：workspace 控制器附近的方法引用 ──
    methods = set()
    for pattern in (
        r"workspaceController\[\\?[\"']([A-Za-z_][A-Za-z0-9_]*)",
        r"workspace\.[A-Za-z_][A-Za-z0-9_]*\s*[,;)}\s]",
        r"['\"](/?api/[a-z0-9\-_./]{2,44})['\"]",
    ):
        methods.update(re.findall(pattern, js))
    ws_methods = sorted({
        m for m in methods
        if isinstance(m, str) and ("workspace" in m.lower() or m[0].isupper())
    })
    result["methodCandidates"] = ws_methods[:60]
    print(f"\n=== 方法/端点候选（{len(ws_methods)}）===")
    for m in ws_methods[:30]:
        print(f"  {m}")

    # ── 3. DOM 锚点：role / aria / data-* 里与工作区相关的 ──
    anchors = set()
    for pattern in (
        r'["\'](data-[a-z0-9\-]*(?:workspace|tree|group|pin|archiv)[a-z0-9\-]*)["\']',
        r'["\'](aria-[a-z]+)["\']',
        r'role:\s*["\']([a-z]+)["\']',
    ):
        anchors.update(re.findall(pattern, js, re.I))
    anchors = {a for a in anchors if isinstance(a, str) and len(a) < 40}
    result["domAnchors"] = sorted(anchors)
    print(f"\n=== DOM 锚点候选（{len(anchors)}）===")
    for a in sorted(anchors):
        print(f"  {a}")

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps({
        "_probe": {
            "note": (
                "dsh-client-ui-workspace / dsh-api-workspace-controller 的 chunk 实测。"
                "「未分组 / 置顶 / 归档」在 DSH 里是真实概念且有 i18n；"
                "但 /api/workspace/* 命名空间猜测全 404 —— 方法名要从这些"
                "上下文里再挖，或者改走 DOM 读法。"
            ),
            "source": "probe_workspace_chunk.py",
            "chunkBytes": len(js),
        },
        **result,
    }, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"\n已写入 {OUT}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
