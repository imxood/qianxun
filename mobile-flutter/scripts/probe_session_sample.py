"""拉一份真实 session/list 样本，落盘给 HTML 原型用。

为什么原型要用真数据
--------------------
用假数据做原型会掩盖真实分布的问题：393 条里 44% 带 contextPressure、
p90 才 53%、标题长度差异巨大、cwd 有长路径。假数据会让看护带和信号轨
「看起来对」，而真数据下才看得出它扛不扛得住。

同时它顺手验掉一件事：**标题在真实数据里有多长**（原型第二行和标题的
截断策略要按这个定，而不是按我想象的定）。

只读，不写回宿主。输出 JSON 到 docs/probe/session-list-sample.json。
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

OUT = Path(__file__).resolve().parent.parent / "docs" / "probe" / "session-list-sample.json"


def call(host: str, port: int, token: str) -> dict[str, Any]:
    body = json.dumps({
        "type": "client-request",
        "rpcId": "sample",
        "method": "session/list",
        "payload": {"args": {"_request": {}}},
    }).encode()
    req = urllib.request.Request(
        f"http://{host}:{port}/api/session/list",
        data=body, method="POST",
        headers={"Cookie": f"qx_token={token}", "Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.loads(resp.read().decode())


def unwrap(raw: dict[str, Any]) -> list[dict[str, Any]]:
    """剥掉三层信封，取出 items 数组。

    实测线格式（`{"type":"server-response", …}`）：
        raw.result.ok.value.items
    失败时：raw.result.ok === false，错误在 raw.result.error。

    这里写成显式的三级下钻而不是 `raw.get("value", raw)` 那种「猜一层」——
    猜错的后果是**安静地返回空列表**，看起来像「部署上 0 条会话」，
    那是一个会被当真好几天的假结论。
    """
    if raw.get("type") != "server-response":
        raise ValueError(f"不是 server-response：type={raw.get('type')!r}")
    result = raw.get("result")
    if not isinstance(result, dict):
        raise ValueError(f"缺 result 字段：{sorted(raw)}")
    if not result.get("ok"):
        raise ValueError(f"RPC 失败：{json.dumps(result.get('error'), ensure_ascii=False)[:300]}")
    value = result.get("value")
    if not isinstance(value, dict) or not isinstance(value.get("items"), list):
        raise ValueError(f"value 里没有 items 数组：{sorted(value)[:10] if isinstance(value, dict) else type(value)}")
    return value["items"]


def text_len(text: str) -> int:
    return len(text)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--host", default="192.168.20.2")
    ap.add_argument("--port", type=int, default=23090)
    ap.add_argument("--token", default="", help="64 位 hex；留空则读 QX_PROBE_TOKEN，再留空则向模拟器取")
    ap.add_argument("--sample", type=int, default=14, help="原型里放多少条")
    args = ap.parse_args()

    # token 依次尝试：命令行 → 环境变量 → 设备。
    # 走环境变量/设备而不是要求用户复制粘贴，是因为 64 位 hex 手工贴必错，
    # 而且命令行参数会留在 shell 历史里。
    token = args.token or os.environ.get("QX_PROBE_TOKEN", "")
    if not token:
        print("未提供 token，改从模拟器取（需已 adb root）…")
        probe = Path(__file__).resolve().parent / "device_token.py"
        if probe.is_file():
            result = subprocess.run(
                [sys.executable, str(probe)], capture_output=True, timeout=90,
            )
            token = (result.stdout or b"").decode("utf-8", "ignore").strip().splitlines()[-1] if result.stdout else ""
            if not token:
                print("设备上也没取到：")
                print((result.stderr or b"").decode("utf-8", "ignore")[:400])
                return 1
        else:
            print("找不到 device_token.py")
            return 1

    try:
        raw = call(args.host, args.port, token)
        items = unwrap(raw)
    except urllib.error.URLError as exc:
        print(f"连接失败: {exc}")
        return 1
    except ValueError as exc:
        # 0 条时不要硬着头皮写空文件 —— 先把信封形状打出来。
        # 探针最容易的失败模式就是「解错了一层，于是安静地报告 0」。
        print(f"解析失败: {exc}")
        print("原始响应前 600 字：")
        print(json.dumps(raw, ensure_ascii=False)[:600])
        return 1

    print(f"总条数 {len(items)}")
    # --- 标题长度分布：决定截断策略 ---
    titles = [
        str((item.get("projections", {}).get("values", {}) or {}).get("title") or "")
        for item in items
    ]
    nonempty = [t for t in titles if t]
    lengths = sorted(text_len(t) for t in nonempty)
    if lengths:
        def pct(p: float) -> int:
            return lengths[min(len(lengths) - 1, int(len(lengths) * p))]
        print("\n--- 标题长度分布 ---")
        print(f"非空 {len(nonempty)}/{len(titles)}")
        print(f"min {lengths[0]} · p50 {pct(0.5)} · p90 {pct(0.9)} · p99 {pct(0.99)} · max {lengths[-1]}")
        for threshold in (20, 30, 40, 60, 80):
            over = sum(1 for n in lengths if n > threshold)
            print(f"  >{threshold} 字: {over:>4} 条 ({over / len(lengths) * 100:.1f}%)")

    # --- cwd 长度：第二行路径占多宽 ---
    cwds = [str(item.get("cwd") or "") for item in items if item.get("cwd")]
    if cwds:
        clens = sorted(text_len(c) for c in cwds)
        print(f"\ncwd: {len(cwds)} 条 · 最长 {clens[-1]} 字 · p90 {clens[int(len(clens) * 0.9)]} 字")
        print(f"  最长样例: {max(cwds, key=text_len)}")

    # --- 状态分布：原型要覆盖的态 ---
    print("\n--- 状态分布 ---")
    running = sum(1 for i in items if i.get("running"))
    detached = 0
    ids = {i.get("sessionId") for i in items}
    for i in items:
        if i.get("origin") == "subagent" and i.get("parentSessionId") not in ids:
            detached += 1
    print(f"running=true: {running}")
    print(f"已断开（子会话∧父不在列表）: {detached}")

    ctx = [
        (i.get("projections", {}).get("values", {}) or {}).get("contextPressure")
        for i in items
    ]
    with_ctx = [c for c in ctx if isinstance(c, dict)]
    print(f"带 contextPressure: {len(with_ctx)} ({len(with_ctx) / max(len(items), 1) * 100:.0f}%)")
    if with_ctx:
        pcts = sorted(
            round(c.get("projectedTokens", 0) / c["contextWindow"] * 100, 1)
            for c in with_ctx if c.get("contextWindow")
        )
        hits = {t: sum(1 for p in pcts if p > t) for t in (50, 65)}
        print(f"  百分比 max {pcts[-1]}% · >50%: {hits[50]} 条 · >65%: {hits[65]} 条")

    q_active = 0
    for i in items:
        uq = (i.get("projections", {}).get("values", {}) or {}).get("userQuestions") or {}
        if isinstance(uq, dict) and uq.get("active"):
            q_active += 1
    print(f"userQuestions.active 非空: {q_active}")

    # --- 取样：优先挑出能覆盖各状态的行 ---
    def score(item: dict[str, Any]) -> tuple:
        values = (item.get("projections", {}).get("values", {}) or {})
        cp = values.get("contextPressure")
        pct = 0.0
        if isinstance(cp, dict) and cp.get("contextWindow"):
            pct = round(cp.get("projectedTokens", 0) / cp["contextWindow"] * 100, 1)
        return (
            0 if item.get("running") else 1,
            -pct,
            item.get("sessionId") or "",
        )

    picked = sorted(items, key=score)[: args.sample]

    slim: list[dict[str, Any]] = []
    for item in picked:
        values = dict((item.get("projections", {}).get("values", {}) or {}))
        outline = values.get("turnOutline") or []
        cp = values.get("contextPressure")
        slim.append({
            "sessionId": item.get("sessionId"),
            "title": values.get("title"),
            "cwd": item.get("cwd"),
            "origin": item.get("origin"),
            "parentSessionId": item.get("parentSessionId"),
            "running": bool(item.get("running")),
            "agentAvailable": item.get("agentAvailable"),
            "updatedAt": item.get("updatedAt"),
            "turnCount": len(outline),
            "lastResponse": (outline[-1].get("response") if outline else None),
            "lastPrompt": (outline[-1].get("prompt") if outline else None),
            "steps": (values.get("sessionStats") or {}).get("steps"),
            "lastTurnCompleted": (values.get("subagentTiming") or {}).get("lastTurnCompleted"),
            "contextPct": (
                round(cp.get("projectedTokens", 0) / cp["contextWindow"] * 100, 1)
                if isinstance(cp, dict) and cp.get("contextWindow") else None
            ),
            "userQuestionsActive": len(
                (values.get("userQuestions") or {}).get("active") or []
            ),
            "queued": len((values.get("inbox") or {}).get("next-turn") or []),
            "teamMembers": len((values.get("agentTeam") or {}).get("members") or []),
            "model": ((values.get("modelSelection") or {}).get("lastUsed") or {}).get("model"),
            "permissions": (values.get("permissions") or {}).get("currentValue"),
        })

    payload = {
        "_probe": {
            "note": (
                "HTML 原型用的真实会话样本。字段是 session/list 原始 item 的**子集**，"
                "缺失字段一律填 null 而非编造。改原型时若需要新字段，"
                "重跑本脚本，不要手填。"
            ),
            "capturedAt": items and None or None,
            "totalSessions": len(items),
            "sampled": len(slim),
            "origin": f"http://{args.host}:{args.port}",
        },
        "items": slim,
    }
    import datetime
    payload["_probe"]["capturedAt"] = datetime.datetime.now(datetime.UTC).isoformat()

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"\n已写入 {OUT}（{len(slim)} 条）")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
