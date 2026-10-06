"""量一条真实会话的转录规模 —— 决定工具卡要不要截断、截多少。

为什么必须量
------------
custom.js 的工具卡要做「两层披露 + 截断 + 全文入口」。截断阈值不能拍脑袋：
  · 太小 → 用户总要点「全文」，那截断就没意义了
  · 太大 → 一次渲染几 MB 文本，手机直接卡死
而实测最长会话是 **72 轮 / 2485 步**，一次 `rg` 输出 2MB（05 §9.5）。

所以这里拉一条真实长会话的分页，统计：
  · 总消息数、总字节数
  · 单条消息/工具输出的**字节分布**（p50/p90/p99/max）
  · 超过 64KB 的有多少条（=「全文」入口的命中率）

用法：
  python scripts/probe_transcript_scale.py                 # 自动挑最长的
  python scripts/probe_transcript_scale.py --session <id>
"""
from __future__ import annotations

import argparse
import json
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

QIANXUN = Path(r"C:\Users\maxu\.qianxun")
GATEWAY = "http://192.168.20.2:23090"
OUT = Path(
    r"E:\develop\dsh-workspace\qianxun\mobile-flutter"
    r"\docs\probe\transcript-scale.json"
)


def token() -> str:
    settings = json.loads((QIANXUN / "settings.json").read_text(encoding="utf-8"))
    return next(d for d in settings["remote"]["devices"] if not d.get("revoked"))["token"]


def call(tok: str, namespace: str, method: str, args: dict[str, Any]) -> dict[str, Any]:
    # ⚠️ 信封里的 `method` 要写**全名** `<ns>/<method>`，不是裸方法名。
    # 写 `list` 会得到
    #   method "list" does not match endpoint "session/list"
    body = json.dumps({
        "type": "client-request",
        "rpcId": "scale",
        "method": f"{namespace}/{method}",
        "payload": {"args": args},
    }).encode()
    req = urllib.request.Request(
        f"{GATEWAY}/api/{namespace}/{method}",
        data=body, method="POST",
        headers={"Cookie": f"qx_token={tok}", "Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=60) as resp:
        payload = json.loads(resp.read().decode())
    result = payload.get("result") or {}
    if result.get("ok") is not True:
        raise RuntimeError(json.dumps(result.get("error"), ensure_ascii=False)[:300])
    return result.get("value") or {}


def pcts(values: list[int], label: str) -> None:
    if not values:
        print(f"  {label}: 无数据")
        return
    ordered = sorted(values)

    def at(p: float) -> int:
        return ordered[min(len(ordered) - 1, int(len(ordered) * p))]

    print(
        f"  {label:<22} n={len(ordered):<5} "
        f"p50={at(0.5):<9} p90={at(0.9):<9} p99={at(0.99):<9} max={ordered[-1]}"
    )


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--session", default="", help="会话 id；留空则自动挑步数最多的")
    ap.add_argument("--max-messages", type=int, default=400)
    args = ap.parse_args()

    tok = token()
    items = call(tok, "session", "list", {"_request": {}})["items"]

    if args.session:
        target = args.session
    else:
        def steps(item: dict[str, Any]) -> int:
            v = (item.get("projections") or {}).get("values") or {}
            s = v.get("sessionStats") or {}
            return s.get("steps") or 0
        best = max(items, key=steps)
        target = best["sessionId"]
        print(f"自动选中步数最多的会话：{steps(best)} 步")

    print(f"会话 {target}\n")
    # ⚠️ `throughSeq` 是**游标**，不是随便一个上限。填 2^31-1 会得到
    #   `session page through seq 2147483647 is past cursor 13487`
    # 所以先从 `session/projections` 或列表里的 `asOfSeq` 拿真实游标。
    as_of = 0
    for item in items:
        if item.get("sessionId") == target:
            as_of = ((item.get("projections") or {}).get("asOfSeq")) or 0
            break
    if as_of <= 0:
        print("拿不到 asOfSeq，退出")
        return 1
    print(f"游标 asOfSeq = {as_of}\n")

    # ⚠️ **参数要包一层 `request`。**
    # 写平铺 `{address, throughSeq, maxMessages}` 会得到
    #   args fields do not match the descriptor:
    #   missing "request"; unexpected "address", "throughSeq", "maxMessages"
    #
    # 这与 `docs/api/dsh-remote-api.md` 里记的
    # `session/page` 参数表**矛盾** —— 文档说它是平铺的。
    # 实测为准：**除 `session/list`（用 `_request`）外，
    # 其余方法都是 `{ request: {…实际参数…} }`**。
    # 待办：修 API 文档（它现在会误导所有照它写的人）。
    value = call(
        tok, "session", "page",
        {
            "request": {
                "address": {"kind": "session", "sessionId": target},
                "throughSeq": as_of,
                "maxMessages": args.max_messages,
            },
        },
    )
    records = value.get("records") or []
    print(f"取回 {len(records)} 条记录（hasMore={value.get('hasMore')}）\n")

    sizes: list[int] = []
    by_type: dict[str, int] = {}
    tool_sizes: list[int] = []
    text_sizes: list[int] = []

    for record in records:
        event = record.get("event") or record
        kind = event.get("type") or "?"
        raw = json.dumps(event, ensure_ascii=False)
        size = len(raw.encode("utf-8"))
        sizes.append(size)
        by_type[kind] = by_type.get(kind, 0) + 1
        if "tool" in kind or "step" in kind:
            tool_sizes.append(size)
        elif "message" in kind:
            text_sizes.append(size)

    print("--- 每条事件字节 ---")
    pcts(sizes, "全部")
    pcts(tool_sizes, "工具/步骤类")
    pcts(text_sizes, "消息类")

    print("\n--- 事件类型分布（按条数）---")
    for kind, count in sorted(by_type.items(), key=lambda kv: -kv[1])[:12]:
        print(f"  {kind:<34} {count}")

    total = sum(sizes)
    print(f"\n总计 {total / 1024:.0f} KB（{total:,} 字节）")

    for threshold in (8, 16, 32, 64, 128, 256):
        over = sum(1 for s in sizes if s > threshold * 1024)
        print(f"  >{threshold:>4} KB: {over:>4} 条 ({over / max(len(sizes), 1) * 100:5.1f}%)")

    # 建议：折叠态显示的行数 —— 依据 p90 与 max 的中间值
    ordered = sorted(sizes)
    p90 = ordered[int(len(ordered) * 0.9)] if ordered else 0
    print(f"\n建议折叠行数：按 p90={p90 / 1024:.0f}KB，取 20 行约 "
          f"{p90 / 1024 * 20 / max(len(ordered), 1):.0f}KB 量级 —— "
          f"超出的走「全文」入口")

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps({
        "_probe": {
            "note": (
                "真实长会话的转录规模。custom.js 的工具卡截断阈值依据它定，"
                "不凭感觉。**只记分布，不含任何会话内容。**"
            ),
            "source": "probe_transcript_scale.py",
            "sessionId": target,
            "records": len(records),
            "hasMore": value.get("hasMore"),
        },
        "totalBytes": total,
        "perEventBytes": {
            "p50": sorted(sizes)[int(len(sizes) * 0.5)] if sizes else 0,
            "p90": p90,
            "p99": sorted(sizes)[int(len(sizes) * 0.99)] if sizes else 0,
            "max": max(sizes) if sizes else 0,
        },
        "eventTypeCounts": by_type,
        "overThreshold": {
            f"{t}KB": sum(1 for s in sizes if s > t * 1024)
            for t in (8, 16, 32, 64, 128, 256)
        },
    }, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"\n已写入 {OUT}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
