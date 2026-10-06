"""查清 projections.values 的字段数分布 —— 探针报出 {1, 18, 19, 20}。

为什么必须查清
--------------
05 §0.2.3 写的是「`projections.values` 有 **20 个字段**」，那是从
`docs/probe/session-list-item.json` **一条样本**数出来的。

而 `probe_l3_desktop.py` 在全量 394 条上跑出 {1, 18, 19, 20} 四种值。
这意味着**「20」不是协议事实，是那条样本的巧合**。

这个区别很要紧：
- 若真是「恒定 20 个」→ 客户端可以按固定结构解析
- 若是「因会话而异」→ 客户端**必须**按缺字段处理，且不能把「缺」当成「零」

后者正是本项目反复吃过的亏（`ImageLimits` 的「缺失 ≠ 0」）。

所以这里逐条列出差异，落到一份可入库的证据文件。
"""
from __future__ import annotations

import json
import urllib.error
import urllib.request
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

QIANXUN = Path(r"C:\Users\maxu\.qianxun")
OUT = Path(
    r"E:\develop\dsh-workspace\qianxun\mobile-flutter"
    r"\docs\probe\projections-fields.json"
)
UNIVERSE = [
    "contextPressure", "contextBreakdown", "imageLimits", "permissions",
    "agentTeam", "inbox", "todos", "plan", "goal", "modelSelection",
    "subagentCatalog", "subagent", "subagentTiming", "sessionListMetadata",
    "sessionStats", "tokenUsage", "turnOutline", "userQuestions", "title",
]


def main() -> int:
    settings = json.loads((QIANXUN / "settings.json").read_text(encoding="utf-8"))
    remote = settings["remote"]
    device = next(d for d in remote["devices"] if not d.get("revoked"))

    body = json.dumps({
        "type": "client-request", "rpcId": "fields", "method": "session/list",
        "payload": {"args": {"_request": {}}},
    }).encode()
    req = urllib.request.Request(
        f"http://{remote['bindIp']}:{remote['port']}/api/session/list",
        data=body, method="POST",
        headers={"Cookie": f"qx_token={device['token']}", "Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        payload = json.loads(resp.read().decode())

    items = payload["result"]["value"]["items"]
    with_values = [
        i for i in items
        if isinstance(i.get("projections"), dict)
        and isinstance(i["projections"].get("values"), dict)
    ]
    print(f"总条数 {len(items)}，带 projections.values {len(with_values)}")

    counts = Counter(len(i["projections"]["values"]) for i in with_values)
    print("\n--- 字段数分布 ---")
    for n, c in sorted(counts.items()):
        print(f"  {n:>2} 个字段: {c:>3} 条 ({c / len(with_values) * 100:.1f}%)")

    present = Counter()
    for item in with_values:
        present.update(item["projections"]["values"].keys())

    print(f"\n--- 字段出现率（{len(with_values)} 条为分母）---")
    for key in UNIVERSE:
        c = present.get(key, 0)
        mark = "✓" if c == len(with_values) else ("·" if c else "✗")
        print(f"  {mark} {key:<22} {c:>3} 条 ({c / len(with_values) * 100:5.1f}%)")

    # 找出「有 values 但只有 1 个字段」的那条 —— 它能说明什么被省略
    thin = [i for i in with_values if len(i["projections"]["values"]) == 1]
    print(f"\n--- 只有 1 个字段的会话：{len(thin)} 条 ---")
    for item in thin[:3]:
        v = item["projections"]["values"]
        print(f"  {item.get('sessionId')[:20]}… origin={item.get('origin')} "
              f"blank={item.get('blank')} values={sorted(v)}")
        if "title" in v:
            print(f"      title={v['title']!r}")

    # 「哪些字段从不在」也要落盘 —— 那是真正的协议事实
    absent = [k for k in UNIVERSE if present.get(k, 0) == 0]
    print(f"\n从未出现的字段：{absent or '（无，19 个都在）'}")

    # 18 与 19 缺了什么
    print("\n--- 18/19 字段的会话缺什么 ---")
    for target in (18, 19):
        rows = [i for i in with_values if len(i["projections"]["values"]) == target]
        if not rows:
            continue
        missing = Counter()
        full = set(rows[0]["projections"]["values"])
        for item in rows:
            missing.update(full - set(item["projections"]["values"]))
        print(f"  {target} 字段（{len(rows)} 条）缺：{dict(missing) or '（相对首条无缺，是多了别的）'}")

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(
        json.dumps(
            {
                "_probe": {
                    "note": (
                        "projections.values 字段数的**全量**实测。05 §0.2.3 写的「20 个」"
                        "来自单条样本，实际是 {1, 18, 19, 20} 四种 —— 客户端必须按缺字段"
                        "处理，且**不能把「缺」当成「零」**。"
                    ),
                    "source": "probe_projection_fields.py",
                    "totalSessions": len(items),
                    "sessionsWithValues": len(with_values),
                },
                "fieldCountDistribution": {str(k): v for k, v in sorted(counts.items())},
                "fieldPresence": {k: present.get(k, 0) for k in UNIVERSE},
                "fieldsNeverSeen": absent,
            },
            ensure_ascii=False,
            indent=2,
        ),
        encoding="utf-8",
    )
    print(f"\n已写入 {OUT}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
