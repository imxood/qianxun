"""从 dsh-api-workspace-controller 的 chunk 里抠出真实的 RPC 端点名。

背景
----
根页面的 link 标签列出了两个「按需 plugin 组合」URL，其中第一个含
`@deepseek-ai/dsh-api-workspace-controller` 与 `@deepseek-ai/dsh-client-ui-workspace`。

而我猜的 `/api/workspace/list` 等端点**全部 404** —— 说明命名空间猜错了。
压缩后**变量名会变形，但字符串字面量会保留**，所以直接从 chunk 里抠
出 `@Remote('…')` 注册的字面量。

这与 DOM 路线相比更可靠：
  · DOM 读法依赖「页面已经渲染出工作区树」，用户不点开侧栏就读不到
  · chunk 字面量是**静态事实**，读一次就够，且能直接当 API 契约

用法：python scripts/probe_workspace_endpoints.py
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
    r"\docs\probe\workspace-endpoints.json"
)


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
        with urllib.request.urlopen(req, timeout=90) as resp:
            return resp.status, resp.read().decode("utf-8", "ignore")
    except urllib.error.HTTPError as exc:
        return exc.code, exc.read(200).decode("utf-8", "ignore")
    except (urllib.error.URLError, OSError) as exc:
        return 0, f"{type(exc).__name__}: {exc}"


def main() -> int:
    status, html = get("/")
    if status != 200:
        print(f"根页面 {status}")
        return 1

    # 取所有 link href（plugin 组合 URL 很长）
    hrefs = re.findall(r'<link[^>]*href="([^"]+)"', html)
    targets = [h for h in hrefs if "workspace" in h.lower()]
    print(f"根页面里含 workspace 的 plugin 组合：{len(targets)} 个")
    for t in targets:
        print(f"  {t[:140]}…")

    found: dict[str, list[str]] = {}
    for href in targets:
        clean = href.replace("&amp;", "&")
        # ⚠️ 那个 href 是**一个** URL，逗号分隔的多个插件名是它的**路径段**。
        #    按逗号切开分别请求会 404 —— 实测踩过。
        #    正确做法：整条 URL 请求一次，拿回来的就是组合后的代码。
        st, js = get("/" + clean.lstrip("./"))
        size = len(js)
        print(f"\n  抓整条 plugin 组合 → {st}  {size} 字节")
        if st != 200 or size < 100:
            print(f"  （{js[:120]}）")
            continue

        # @Remote('…') / @Remote({name:'…'}) 注册的字面量
        names = set()
        for pattern in (
            r"@Remote\(\s*'([^']+)'",
            r'@Remote\(\s*"([^"]+)"',
            r"@Remote\(\s*\{\s*name:\s*['\"]([^'\"]+)['\"]",
            r"remote\(\s*'([^']+)'",
        ):
            names.update(re.findall(pattern, js))
        if names:
            print(f"    @Remote 注册：{sorted(names)}")

        # 端点路径字面量
        paths = set(re.findall(r"['\"](/?api/[a-z0-9\-_./]+)['\"]", js))
        if paths:
            print(f"    端点路径字面量：{sorted(paths)[:24]}")

        # workspace 相关的命名空间线索
        ns = set(re.findall(r"['\"]([a-z][a-z0-9\-]{2,24})/[a-z][a-zA-Z]{2,20}['\"]", js))
        wsns = {n for n in ns if "work" in n or "space" in n}
        if wsns:
            print(f"    workspace 命名空间候选：{sorted(wsns)}")

        # workspace 相关的字面量
        ws = set(re.findall(r"['\"]([A-Za-z][A-Za-z0-9_\-]{2,40})['\"]", js))
        ws = {w for w in ws if any(
            h in w.lower() for h in ("workspace", "ungroup", "sessionids", "pinned", "archived")
        )}
        if ws:
            print(f"    workspace 相关字面量：{sorted(ws)[:30]}")

        found[clean[:80]] = sorted(names)

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps({
        "_probe": {
            "note": (
                "从 dsh-api-workspace-controller 的 chunk 里抠出的 @Remote 注册名。"
                "猜 /api/workspace/list 全部 404 —— 命名空间不是 'workspace'。"
            ),
            "source": "probe_workspace_endpoints.py",
        },
        "remoteRegistrations": found,
    }, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"\n已写入 {OUT}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
