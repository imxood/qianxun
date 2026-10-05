#!/usr/bin/env python3
"""上下文占用的**真实分布** —— 用来裁决 C10 的百分比阈值。

为什么必须先量再定阈值
----------------------
约束书 C10 写的是「<70% 不显示 / 70–85% 灰字 / >85% 琥珀 + 条」。
但那是**拍出来的**数字。实测 393 条会话里最大的一条只有
`465695 / 1048576 ≈ 44%`（`probe_context_calibration.py` 的输出）。

如果全量分布的峰值远低于 70%，那么：
  - 阈值形同虚设，meter 永远不出现；
  - 更好���情况是它在**唯一那条快满了**的会话上不出现 —— 而那正是它
    唯一该出现的时刻。
这与「状态必须标注新鲜度」是同一类错误：**设计了一个从不触发的状态。**

所以这里输出完整分布，让阈值由数据决定。
"""

from __future__ import annotations

import argparse
import json
import urllib.error
import urllib.request
from collections import Counter


def call(host: str, port: int, token: str) -> dict:
    body = json.dumps({
        "type": "client-request",
        "rpcId": "ctx-dist",
        "method": "session/list",
        "payload": {"args": {"_request": {}}},
    }).encode()
    req = urllib.request.Request(
        f"http://{host}:{port}/api/session/list",
        data=body, method="POST",
        headers={"Cookie": f"qx_token={token}", "Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(req, timeout=25) as r:
            return json.loads(r.read().decode("utf-8", "replace"))
    except urllib.error.HTTPError as e:
        return {"_http": e.code, "_body": e.read().decode("utf-8", "replace")[:300]}
    except Exception as e:  # noqa: BLE001
        return {"_err": f"{type(e).__name__}: {e}"}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--token", required=True)
    ap.add_argument("--host", default="192.168.20.2")
    ap.add_argument("--port", type=int, default=23090)
    args = ap.parse_args()

    res = call(args.host, args.port, args.token)
    if "_err" in res or "_http" in res:
        print(json.dumps(res, ensure_ascii=False, indent=2))
        return 1

    items = res.get("result", {}).get("value", {}).get("items", [])
    rows = []
    for it in items:
        v = (it.get("projections") or {}).get("values") or {}
        pr = v.get("contextPressure") or {}
        used = pr.get("projectedTokens")
        win = pr.get("contextWindow")
        if isinstance(used, int) and isinstance(win, int) and win > 0:
            rows.append({
                "sid": (it.get("sessionId") or "")[:16],
                "pct": used / win * 100,
                "used": used,
                "win": win,
                "updated": it.get("updatedAt"),
            })

    total = len(items)
    if not rows:
        print(f"{total} 条会话，但**没有一条带 contextPressure** —— "
              f"阈值门控无从谈起，先查数据源。")
        return 0

    pcts = sorted(r["pct"] for r in rows)
    n = len(pcts)

    def q(f: float) -> float:
        return pcts[min(n - 1, int(n * f))]

    print(f"会话总数          : {total}")
    print(f"带 contextPressure: {n}  ({n / total:.0%})")
    print()
    print("占用率分布（升序）：")
    for f, label in ((0, "min"), (0.25, "p25"), (0.5, "中位"),
                     (0.75, "p75"), (0.9, "p90"), (0.99, "p99")):
        print(f"  {label:<5} {q(f):6.1f}%")
    print(f"  max   {pcts[-1]:6.1f}%")
    print()

    bands = Counter()
    for r in rows:
        p = r["pct"]
        band = ("<20%" if p < 20 else "20–40%" if p < 40 else
                "40–60%" if p < 60 else "60–70%" if p < 70 else
                "70–85%" if p < 85 else "85%+")
        bands[band] += 1
    order = ["<20%", "20–40%", "40–60%", "60–70%", "70–85%", "85%+"]
    print("分桶：")
    for b in order:
        c = bands.get(b, 0)
        bar = "█" * int(c / max(1, n) * 40)
        print(f"  {b:<8} {c:>4}  {bar}")

    print()
    for thr in (0.40, 0.50, 0.60, 0.70, 0.85):
        c = sum(1 for p in pcts if p >= thr * 100)
        verdict = ("❌ 从不触发" if c == 0 else
                   "❌ 几乎不触发" if c / n < 0.02 else
                   "⚠️ 少见" if c / n < 0.10 else
                   "✓ 有意义")
        print(f"阈值 {thr:.0%} → 命中 {c:>4} 条 ({c / n:6.1%})  {verdict}")

    print()
    print("占用最高的 5 条：")
    for r in sorted(rows, key=lambda x: -x["pct"])[:5]:
        print(f"  {r['sid']:<18} {r['pct']:5.1f}%  "
              f"({r['used']:,} / {r['win']:,})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
