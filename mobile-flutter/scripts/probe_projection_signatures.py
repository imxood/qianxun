"""查清 projections.values 为什么按 186 / 84 / 121 分组。

上一支探针的发现
----------------
`projections.values` 的字段数在 394 条上呈 {1, 18, 19, 20}：

    1 个字段  : 208 条 (52.8%)  ← 只有 title
    18 个字段 :  65 条 (16.5%)
    19 个字段 :  37 条 ( 9.4%)
    20 个字段 :  84 条 (21.3%)

且 `userQuestions` 只在 84 条（=20 字段那组）里出现，
`agentTeam` 在 121 条（= 84 + 37，也就是 20 和 19 那两组）。

**这组数字太整齐了，像是按「会话有没有在跑 / 有没有过轮次」分的桶。**
如果真是那样，那么「缺字段」的语义是确定的，且有规律 —— 那就可以写进
客户端的解析规则。若不查清，客户端只能把 208 条空壳会话也当成
「有 18 个字段但它们都是零」，进而画出错误的上下文计量与状态。

所以这里按「字段签名」分组，看每组对应会话的什么特征。
"""
from __future__ import annotations

import json
import urllib.request
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

QIANXUN = Path(r"C:\Users\maxu\.qianxun")
OUT = Path(
    r"E:\develop\dsh-workspace\qianxun\mobile-flutter"
    r"\docs\probe\projection-signatures.json"
)
CANDIDATES = [
    "contextPressure", "contextBreakdown", "imageLimits", "permissions",
    "agentTeam", "inbox", "todos", "plan", "goal", "modelSelection",
    "subagentCatalog", "subagent", "subagentTiming", "sessionListMetadata",
    "sessionStats", "tokenUsage", "turnOutline", "userQuestions", "title",
]


def fetch() -> list[dict[str, Any]]:
    settings = json.loads((QIANXUN / "settings.json").read_text(encoding="utf-8"))
    remote = settings["remote"]
    device = next(d for d in remote["devices"] if not d.get("revoked"))
    body = json.dumps({
        "type": "client-request", "rpcId": "sig", "method": "session/list",
        "payload": {"args": {"_request": {}}},
    }).encode()
    req = urllib.request.Request(
        f"http://{remote['bindIp']}:{remote['port']}/api/session/list",
        data=body, method="POST",
        headers={"Cookie": f"qx_token={device['token']}", "Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.loads(resp.read().decode())["result"]["value"]["items"]


def main() -> int:
    items = fetch()
    groups: dict[tuple[str, ...], list[dict[str, Any]]] = defaultdict(list)
    for item in items:
        values = (item.get("projections") or {}).get("values") or {}
        groups[tuple(sorted(values))].append(item)

    print(f"按字段签名分组：{len(groups)} 组\n")
    signature_report = []
    for signature, rows in sorted(groups.items(), key=lambda kv: -len(kv[1])):
        n = len(rows)
        present = set(signature)
        absent = [k for k in CANDIDATES if k not in present]
        print(f"── 组 {n} 条 ({n / len(items) * 100:.1f}%) · {len(signature)} 字段")
        print(f"   有：{', '.join(signature[:6])}{' …' if len(signature) > 6 else ''}")
        if absent:
            print(f"   缺：{', '.join(absent)}")

        # 这一组里的会话有什么共同特征？
        running = sum(1 for r in rows if r.get("running"))
        origin = Counter(str(r.get("origin")) for r in rows)
        stats = [((r.get("projections") or {}).get("values") or {}).get("sessionStats") for r in rows]
        with_stats = [s for s in stats if isinstance(s, dict)]
        turns = [s.get("turns") for s in with_stats if isinstance(s.get("turns"), int)]
        steps = [s.get("steps") for s in with_stats if isinstance(s.get("steps"), int)]
        # 用户消息条数（turnOutline）也是「有没有内容」的信号
        outlines = [len((r["projections"]["values"].get("turnOutline") or [])) for r in rows if "turnOutline" in present]
        timing = [
            ((r.get("projections") or {}).get("values") or {}).get("subagentTiming")
            for r in rows
        ]
        with_timing = [t for t in timing if isinstance(t, dict)]
        completed = sum(1 for t in with_timing if t.get("lastTurnCompleted") is True)

        print(f"   running=true {running} | origin {dict(origin)}")
        if turns:
            print(f"   turns  min {min(turns)} p50 {sorted(turns)[len(turns)//2]} max {max(turns)}")
        if steps:
            print(f"   steps  min {min(steps)} p50 {sorted(steps)[len(steps)//2]} max {max(steps)}")
        if outlines:
            print(f"   turnOutline 条数 min {min(outlines)} p50 {sorted(outlines)[len(outlines)//2]} max {max(outlines)}")
        if with_timing:
            print(f"   有 subagentTiming {len(with_timing)}，其中 lastTurnCompleted=true {completed}")
        print()

        signature_report.append({
            "count": n,
            "fieldCount": len(signature),
            "fields": list(signature),
            "absent": absent,
            "runningTrue": running,
            "origin": dict(origin),
            "turnsRange": [min(turns), max(turns)] if turns else None,
            "stepsRange": [min(steps), max(steps)] if steps else None,
            "turnOutlineLengthRange": [min(outlines), max(outlines)] if outlines else None,
            "hasSubagentTiming": len(with_timing),
            "lastTurnCompletedTrue": completed,
        })

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(
        json.dumps(
            {
                "_probe": {
                    "note": (
                        "projections.values 的**字段签名**分组。05 §0.2.3 记的「20 个字段」"
                        "只是 4 个签名之一（84/394 = 21.3%），且 52.8% 的会话只有 title。"
                        "客户端必须按签名而非固定结构解析。"
                    ),
                    "source": "probe_projection_signatures.py",
                    "totalSessions": len(items),
                },
                "signatures": signature_report,
            },
            ensure_ascii=False,
            indent=2,
        ),
        encoding="utf-8",
    )
    print(f"已写入 {OUT}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
