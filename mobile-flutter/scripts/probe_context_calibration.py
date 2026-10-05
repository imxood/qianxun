#!/usr/bin/env python3
"""对比 `contextPressure` 与 `contextBreakdown` 两个口径。

为什么必须验
------------
设计提案 A 在 `docs/design-proposals/A-information-architecture.md` §10(3)
指出：一条 item 里 `contextBreakdown` 三段求和 = 80878，而
`contextPressure.pressureTokens` = 98478，**差 21.8%**。

若我们把「上下文 87%」这个阈值建在**错误的口径**上，门槛就会系统性偏移：
用户看到的百分比与服务端自己的判断不一致 —— 而「上下文」正是那种
**用户会拿它做决定**的数字（要不要开新会话、要不要压缩）。

本脚本在**多条不同长度的会话**上同时读两个口径，看差值是否稳定
（= 两个不同的量）还是随机（= 解析错位）。
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
        "rpcId": "ctx-probe",
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
    ap.add_argument("--top", type=int, default=12, help="看最大的 N 条")
    args = ap.parse_args()

    res = call(args.host, args.port, args.token)
    if "_err" in res or "_http" in res:
        print(json.dumps(res, ensure_ascii=False, indent=2))
        return 1

    items = res.get("result", {}).get("value", {}).get("items", [])
    print(f"会话总数 {len(items)}\n")

    rows = []
    for it in items:
        v = (it.get("projections") or {}).get("values") or {}
        bd = v.get("contextBreakdown") or {}
        pr = v.get("contextPressure") or {}
        if not bd and not pr:
            continue
        seg = sum(x for x in (
            bd.get("systemTokens"), bd.get("toolsTokens"), bd.get("messageTokens"),
        ) if isinstance(x, int))
        rows.append({
            "sid": (it.get("sessionId") or "")[:14],
            "seg": seg,
            "pressure": pr.get("pressureTokens"),
            "window": pr.get("contextWindow"),
            "raw_bd": bd,
            "raw_pr": pr,
        })

    if not rows:
        print("没有会话带上下文数据")
        return 0

    rows.sort(key=lambda r: -(r["pressure"] or 0))
    print(f"带上下文数据的会话 {len(rows)} 条，按 pressure 降序取前 {min(args.top, len(rows))}：\n")
    print(f"{'session':<16} {'三段求和':>10} {'pressure':>10} {'差':>8} {'差%':>7} {'window':>9}")
    diffs = []
    for r in rows[: args.top]:
        seg, pr = r["seg"], r["pressure"]
        if isinstance(seg, int) and isinstance(pr, int) and pr > 0:
            d = pr - seg
            pct = d / pr * 100
            diffs.append(pct)
            print(f"{r['sid']:<16} {seg:>10} {pr:>10} {d:>8} {pct:>6.1f}% {r['window'] or '-':>9}")
        else:
            print(f"{r['sid']:<16} {seg:>10} {str(pr):>10} {'-':>8} {'-':>7} {r['window'] or '-':>9}")

    if diffs:
        print()
        print(f"差值占 pressure 的比例：min {min(diffs):.1f}%  max {max(diffs):.1f}%  "
              f"平均 {sum(diffs)/len(diffs):.1f}%")
        spread = max(diffs) - min(diffs)
        if spread < 3:
            print("\n→ 差值**稳定**，说明两者是「不同的量」（例如 pressure 含输出/预留）")
            print("  百分比阈值必须用 pressureTokens / contextWindow（只有它给分母）。")
        else:
            print("\n→ 差值**不稳定**，倾向于解析错位而非口径差异，**实现前必须停下查**。")
    else:
        print("\n没有可对比的样本（两者缺一）")

    # 原始形状，便于判断是不是还有我们没读到的字段
    print("\n=== 一条样本的原始字段 ===")
    print(json.dumps(rows[0]["raw_bd"], ensure_ascii=False))
    print(json.dumps(rows[0]["raw_pr"], ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
