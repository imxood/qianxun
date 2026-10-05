#!/usr/bin/env python3
"""把**一条 session/list 的原始 item** 落盘到 docs/probe/。

为什么必须留档
--------------
审计发现（docs/research/02-protocol-audit.md §5）：三个既有探针都只
`print` 到 stdout，**仓库里没有任何一份实测输出**。于是 §0.2「已验证的
事实」那张表全靠注释转述 —— 任何人重写客户端时无法复核，重构也没有
回归保护。同一笔探测的成本被迫付了第二次。

本脚本把原始 JSON **原样**写进 `docs/probe/session-list-item.json`，
并在旁边放一份人可读的摘要。**不改写、不裁剪、不美化** —— 美化过的
证据不再是证据。

这一条能同时验掉审计 §4.1 / 4.2 / 4.3 / 4.4 / 4.6 五个未验证项：
`userQuestions` 的层级与键名、`turnOutline` 的层级、`agentAvailable`
的真实取值、`permissions.currentValue` 是否存在、`agentId` 与
`sessionId` 的归属关系。

用法::

    python scripts/probe_session_list_item.py --token <64位hex>
    python scripts/probe_session_list_item.py --token <hex> --session <id>  # 盯住某一条
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT_DIR = ROOT / "docs" / "probe"

# 与 lib/data/dsh/ 的实测定稿一致（docs/research/01-facts.md §1b）。
# 只有一个方法用 `_request`，其余一律 `request`。
ARG_KEY = {"session/list": "_request"}


def call(host: str, port: int, token: str, method: str, args: dict) -> dict:
    body = json.dumps(
        {
            "type": "client-request",
            "rpcId": "probe-item",
            "method": method,
            "payload": {"args": {ARG_KEY.get(method, "request"): args}},
        }
    ).encode()
    req = urllib.request.Request(
        f"http://{host}:{port}/api/{method}",
        data=body,
        method="POST",
        headers={
            "Cookie": f"qx_token={token}",
            "Content-Type": "application/json",
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=25) as r:
            return json.loads(r.read().decode("utf-8", "replace"))
    except urllib.error.HTTPError as e:
        return {"_http_error": e.code, "_body": e.read().decode("utf-8", "replace")[:500]}
    except Exception as e:  # noqa: BLE001
        return {"_error": f"{type(e).__name__}: {e}"}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--token", required=True)
    ap.add_argument("--host", default="192.168.20.2")
    ap.add_argument("--port", type=int, default=23090)
    ap.add_argument("--session", default="", help="只保留这条会话的 item")
    args = ap.parse_args()

    res = call(args.host, args.port, args.token, "session/list", {})
    if "_error" in res or "_http_error" in res:
        print(json.dumps(res, ensure_ascii=False, indent=2), file=sys.stderr)
        return 1

    items = res.get("result", {}).get("value", {}).get("items", [])
    if args.session:
        items = [i for i in items if i.get("sessionId") == args.session]
    if not items:
        print("没有会话可 dump", file=sys.stderr)
        return 1

    # 挑信息量最大的那条：字段最多的优先，其次更新时间最近的。
    def richness(it: dict) -> tuple[int, int]:
        return (len(it), it.get("updatedAt") or 0)

    item = max(items, key=richness)
    payload = {
        "_probe": {
            "capturedAt": datetime.now(timezone.utc).isoformat(timespec="seconds"),
            "method": "session/list",
            "argKey": ARG_KEY["session/list"],
            "origin": f"http://{args.host}:{args.port}",
            "totalSessionsInResponse": len(items),
            "note": (
                "原始响应未做任何改动。若日后 §0.2 的某条声明被推翻，"
                "请重跑 probe_session_list_item.py 并覆盖本文件，"
                "同时改 05-product-design.md §0.2 —— 不要只改代码注释。"
            ),
        },
        "item": item,
    }

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    out = OUT_DIR / "session-list-item.json"
    out.write_text(
        json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(f"已写入 {out}  ({out.stat().st_size} 字节)")

    # ── 人可读摘要：逐条回答审计提出的未验证问题 ──
    print("\n=== 这条 item 回答了什么 ===")
    proj = (item.get("projections") or {}).get("values") or {}
    print(f"顶层字段: {sorted(item.keys())}")
    print(f"projections.values 字段: {sorted(proj.keys())}")

    for key, where in (
        ("userQuestions", "顶层 / projections.values"),
        ("turnOutline", "顶层 / projections.values"),
        ("permissions", "顶层 / projections.values"),
        ("agentAvailable", "顶层"),
        ("parentSessionId", "顶层"),
    ):
        top = key in item
        inn = key in proj
        verdict = "顶层" if top else ("projections.values" if inn else "❌ 两处都没有")
        detail = ""
        if top:
            v = item[key]
            detail = f"  键: {sorted(v.keys()) if isinstance(v, dict) else type(v).__name__}"
            if isinstance(v, dict) and "currentValue" in v:
                detail += f"  currentValue={v['currentValue']!r}"
        elif inn:
            v = proj[key]
            detail = f"  值: {str(v)[:80]}"
        print(f"  {key:<18} → {verdict}{detail}")

    av = item.get("agentAvailable")
    print(f"\nagentAvailable 本条取值: {av!r}（审计 §4.3：全表 392 False / 1 True）")
    print(f"parentSessionId 本条取值: {item.get('parentSessionId')!r}"
          f"  → {'是子会话' if item.get('parentSessionId') else '顶级会话'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
