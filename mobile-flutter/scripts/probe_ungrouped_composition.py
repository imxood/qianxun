"""算清楚「未登记的 251 条」到底是什么 —— 它们是不是大部分都是子 agent 会话。

为什么必须算这个
----------------
用户批的是「把 251 条未归属会话做成一个『未分组』区，默认收起」。
但在动手前发现 `session/list` 每条还带两个字段：

    parentSessionId   250/394 有
    origin            249/394 有，样本值 "subagent"

也就是说，那 251 条里可能**大部分根本不是用户建的会话，而是 agent 自己开的
子会话**。如果是这样，补一个「未分组」区等于把一屏子 agent 日志端到用户面前 ——
是噪声不是价值。先算清楚再决定。

注册表在本地文件里（Python 能读），RPC 在页面里（浏览器能读），
所以这个 join 只能在 Python 侧做：先拿注册表的 143 个 id，
再按 origin / parentSessionId 分类 394 条。

输出：docs/probe/ungrouped-composition.json
"""
from __future__ import annotations

import json
import sys
import urllib.error
import urllib.request
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
QIANXUN = Path(r"C:\Users\maxu\.qianxun")
DSH_HOME = QIANXUN / "dsh-home"
GATEWAY = "http://192.168.20.2:23090"
OUT = ROOT / "docs" / "probe" / "ungrouped-composition.json"


def token() -> str:
    settings = json.loads((QIANXUN / "settings.json").read_text(encoding="utf-8"))
    return next(d for d in settings["remote"]["devices"] if not d.get("revoked"))["token"]


def rpc(path: str, method: str, args: dict, key: str) -> dict:
    req = urllib.request.Request(
        f"{GATEWAY}{path}",
        data=json.dumps({
            "type": "client-request",
            "rpcId": "probe",
            "method": method,
            "payload": {"args": {key: args}},
        }).encode(),
        headers={"Content-Type": "application/json", "Cookie": f"qx_token={token()}"},
    )
    with urllib.request.urlopen(req, timeout=120) as resp:
        return json.loads(resp.read().decode("utf-8"))


def registry_ids() -> tuple[set[str], dict]:
    """注册表认领的 sessionId 集合 + 原文件摘要。"""
    path = DSH_HOME / "storages" / "workspace.json"
    data = json.loads(path.read_text(encoding="utf-8"))
    tables = data.get("tables", {})
    claimed: set[str] = set()
    per_ws: dict[str, int] = {}
    for ws_id, ws in (tables.get("workspaces") or {}).items():
        ids = ws.get("sessionIds") or []
        per_ws[ws.get("title") or ws_id] = len(ids)
        claimed.update(ids)
    glob = tables.get("global") or {}
    summary = {
        "workspaceCount": len(per_ws),
        "perWorkspace": per_ws,
        "claimed": len(claimed),
        "archivedSessionIds": len(glob.get("archivedSessionIds") or []),
        "pinnedSessionIds": len(glob.get("pinnedSessionIds") or []),
    }
    return claimed, summary


def main() -> int:
    claimed, reg = registry_ids()
    body = rpc("/api/session/list", "session/list", {}, "_request")
    items = body["result"]["value"]["items"]
    ids = [it["sessionId"] for it in items]

    def classify(it: dict) -> str:
        sid = it["sessionId"]
        if sid in claimed:
            return "registered"
        if it.get("parentSessionId") or it.get("origin") == "subagent":
            return "unregistered_but_subagent"
        return "unregistered_and_not_subagent"

    buckets: dict[str, list[dict]] = {}
    for it in items:
        buckets.setdefault(classify(it), []).append(it)

    origins = Counter(str(it.get("origin")) for it in items)
    id_shapes = Counter(
        "session-前缀" if sid.startswith("session-") else "裸 uuid"
        for sid in ids
    )

    summary = {}
    for name, group in buckets.items():
        summary[name] = {
            "count": len(group),
            "样例标题": [
                (it.get("projections", {}).get("values", {}) or {}).get("title")
                for it in group[:4]
            ],
            "样例sessionId": [it["sessionId"] for it in group[:3]],
            "cwd分布": dict(Counter(str(it.get("cwd")) for it in group).most_common(4)),
        }

    result = {
        "sessionTotal": len(items),
        "registry": reg,
        "分桶": summary,
        "origin分布": dict(origins),
        "sessionId形态": dict(id_shapes),
    }
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")

    print(f"session/list 共 {len(items)} 条，注册表认领 {len(claimed)} 条\n")
    for name, s in summary.items():
        print(f"  {name:<32} {s['count']:>4}")
        print(f"      cwd: {s['cwd分布']}")
        print(f"      标题样例: {s['样例标题']}")
    print(f"\norigin 分布: {dict(origins)}")
    print(f"sessionId 形态: {dict(id_shapes)}")
    print(f"→ {OUT.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
