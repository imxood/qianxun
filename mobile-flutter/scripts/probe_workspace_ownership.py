"""工作区归属：显式注册表 vs cwd 推断 —— 两者**不等价**，且差 64%

为什么必须先查清
----------------
第 2 版的 `custom.js` 按 `session/list` 的 `cwd` 分组，得到 8 个「工作区」。
但用户给的截图里 DSH 侧栏只有 **6 个**工作区。差在哪？

实测结论（scripts/probe_workspace_ownership.py）：

    session/list 总数                 394
    注册表登记的会话数               143
    两者都有                         143
    **只在 list 里（未注册）**      251  (63.7%)

注册表在 `<dsh_home>/storages/workspace.json`：

    tables.workspaces[<id>] = { path, title, sessionIds[], createdAt, updatedAt }
    global = { workspaceIds[6], archivedSessionIds[3], pinnedSessionIds[] }

**所以「工作区」有两个概念，之前的实现搞错了：**

| | DSH 侧栏显示的 | 我第 2 版造的 |
|---|---|---|
| 来源 | `workspace.json` 的 `tables.workspaces` | `session/list` 的 `cwd` 分组 |
| 条数 | **6** | **8** |
| 归属依据 | `sessionIds` **显式列表** | `cwd` 字符串**推断** |
| 覆盖 | 143 条 | 394 条 |

`cwd` 推断会**凭空多造 2 个工作区**（`ai_test` 1 条、`develop` 1 条），
而且把 251 条「不属于任何工作区」的会话硬塞进按目录猜的组里。

⚠️ 关键后果：那 251 条在 DSH 侧栏里是**「未分组」区**（截图最下面
「你好你是什么模型 / AVX2与AVX-512含义及用途 / …/ 展开其余 18 个会话」
就是这一区）。把它们按 cwd 分组展示，等于**在手机上伪造了 DSH 里
不存在的层级**。
"""
from __future__ import annotations

import json
import urllib.request
from collections import Counter
from pathlib import Path
from typing import Any

QIANXUN = Path(r"C:\Users\maxu\.qianxun")
DSH_HOME = QIANXUN / "dsh-home"
REGISTRY = DSH_HOME / "storages" / "workspace.json"
OUT = Path(
    r"E:\develop\dsh-workspace\qianxun\mobile-flutter"
    r"\docs\probe\workspace-ownership.json"
)
GATEWAY = "http://192.168.20.2:23090"


def token() -> str:
    settings = json.loads((QIANXUN / "settings.json").read_text(encoding="utf-8"))
    return next(
        d for d in settings["remote"]["devices"] if not d.get("revoked")
    )["token"]


def list_sessions() -> list[dict[str, Any]]:
    body = json.dumps({
        "type": "client-request", "rpcId": "ws",
        "method": "session/list", "payload": {"args": {"_request": {}}},
    }).encode()
    req = urllib.request.Request(
        f"{GATEWAY}/api/session/list", data=body, method="POST",
        headers={"Cookie": f"qx_token={token()}", "Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        payload = json.loads(resp.read().decode())
    return payload["result"]["value"]["items"]


def main() -> int:
    registry = json.loads(REGISTRY.read_text(encoding="utf-8"))
    tables = registry["tables"]["workspaces"]
    items = list_sessions()

    # 显式归属表
    owner: dict[str, str] = {}
    for wid, ws in tables.items():
        for sid in (ws.get("sessionIds") or []):
            owner[sid] = wid

    owned = [i for i in items if i["sessionId"] in owner]
    unowned = [i for i in items if i["sessionId"] not in owner]

    # 路径 → 标题（注册表里 title 才是显示名，name 字段不存在）
    by_id = tables
    print(f"session/list 总数        : {len(items)}")
    print(f"注册表登记的会话数      : {len(owner)}")
    print(f"两边都有                : {len(owned)}")
    print(f"只在 list 里（未注册）  : {len(unowned)} "
          f"({len(unowned) / len(items) * 100:.1f}%)")

    print("\n=== 6 个注册工作区（按 path，title 才是显示名）===")
    for wid, ws in by_id.items():
        n = len(ws.get("sessionIds") or [])
        listed = sum(1 for i in owned if owner[i["sessionId"]] == wid)
        print(f"  {str(ws.get('title')):<24} {ws.get('path'):<44} "
              f"登记 {n} · 在列表里 {listed}")

    print("\n=== 未注册那 251 条按 cwd 分布（**它们在 DSH 里是「未分组」**）===")
    counter = Counter(
        (i.get("cwd") or "(无 cwd)").replace("\\", "/").rstrip("/").split("/")[-1]
        for i in unowned
    )
    for name, n in counter.most_common(12):
        print(f"  {name:<28} {n}")

    # 归档与置顶
    print("\n=== 注册表全局字段 ===")
    g = registry["global"]
    print(f"  workspaceIds      : {len(g.get('workspaceIds') or [])}")
    print(f"  archivedSessionIds: {len(g.get('archivedSessionIds') or [])}")
    print(f"  pinnedSessionIds  : {len(g.get('pinnedSessionIds') or [])}"
          f"   ← 本机实测为空，所以「置顶」在手机端没有数据源")

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps({
        "_probe": {
            "note": (
                "工作区归属实测。**显式注册表 ≠ cwd 推断**："
                "注册表 6 个工作区 / 143 条，session/list 394 条，"
                "其中 251 条（63.7%）不属于任何工作区 —— DSH 侧栏把它们"
                "放在「未分组」区。按 cwd 分组会凭空多造工作区并伪造层级。"
            ),
            "source": "probe_workspace_ownership.py",
        },
        "totalSessions": len(items),
        "registeredSessions": len(owner),
        "unownedSessions": len(unowned),
        "workspaces": [
            {
                "id": wid,
                "title": ws.get("title"),
                "path": ws.get("path"),
                "registeredSessionIds": len(ws.get("sessionIds") or []),
            }
            for wid, ws in by_id.items()
        ],
        "unownedByCwd": dict(counter.most_common()),
        "global": {
            "workspaceIds": len(g.get("workspaceIds") or []),
            "archivedSessionIds": len(g.get("archivedSessionIds") or []),
            "pinnedSessionIds": len(g.get("pinnedSessionIds") or []),
        },
    }, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"\n已写入 {OUT}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
