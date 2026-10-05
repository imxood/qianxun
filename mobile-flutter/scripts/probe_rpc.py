#!/usr/bin/env python3
"""探测 DSH 网关的真实 RPC 线格式。

为什么需要这个脚本：参考项目（Kotlin 客户端）锁在 harness 0.1.2-alpha.1，
它的载荷是**摊平**的；实际部署（qianxun 0.7.0 内置的 harness）要求
`{"args": {...}}` 信封，且信封内的键要逐字匹配远端 descriptor。
参数名并不统一（`session/list` 是 `_request`，其余是 `request`），
这些**猜不得**，只能打一发看它说什么。

用法::

    python scripts\\probe_rpc.py --token <64位hex> [--host 127.0.0.1] [--port 23090]
"""

from __future__ import annotations

import argparse
import json
import sys
import urllib.error
import urllib.request

# 用 json 拼装再发出去——手工字符串拼接过一次，被 PowerShell 的引号规则
# 搞成了 'body is not JSON'，那次排查纯属自找麻烦。
def _post(host: str, port: int, token: str, method: str, args: dict, clip: int) -> str:
    body = json.dumps(
        {
            "type": "client-request",
            "rpcId": "probe-1",
            "method": method,
            "payload": {"args": args},
        }
    ).encode()
    req = urllib.request.Request(
        f"http://{host}:{port}/api/{method}",
        data=body,
        method="POST",
        headers={
            "Cookie": f"qx_token={token}",
            "Content-Type": "application/json",
            "User-Agent": "QxRpcProbe/1.0",
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            raw = resp.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        raw = f"HTTP {e.code}: {e.read().decode('utf-8', 'replace')}"
    except Exception as e:  # noqa: BLE001
        raw = f"{type(e).__name__}: {e}"
    return raw if len(raw) <= clip else raw[:clip] + " …"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=23090)
    ap.add_argument("--token", required=True)
    ap.add_argument("--clip", type=int, default=420)
    args = ap.parse_args()

    # 1. 先拿一个真实 sessionId，后面的探测要用。
    print("=== 0. 先用 session/list(_request) 拿一个 sessionId ===")
    out = _post(args.host, args.port, args.token, "session/list", {"_request": {}}, 10_000)
    try:
        items = json.loads(out)["result"]["value"]["items"]
    except Exception:  # noqa: BLE001
        print(out)
        return 1
    sid = items[0]["sessionId"] if items else ""
    print(f"  拿到 {len(items)} 个会话，sessionId={sid}")

    addr = {"kind": "session", "sessionId": sid}
    cases: list[tuple[str, dict]] = [
        # 无参方法
        ("session/modelCatalog", {}),
        ("agentPresets/list", {}),
        # 单参方法：request / _request 两种命名各试一次
        ("session/list", {"_request": {}}),
        ("session/list", {"request": {}}),
        ("session/page", {"request": {"address": addr, "throughSeq": -1, "maxMessages": 3}}),
        ("session/cancel", {"request": {"sessionId": sid}}),
        ("skills/list", {"request": {"sessionId": sid}}),
        # 故意写错，用来确认错误文案长什么样
        ("session/list", {"args": {}}),
    ]

    for method, a in cases:
        print(f"\n=== {method}  args={json.dumps(a, ensure_ascii=False)[:120]} ===")
        print(_post(args.host, args.port, args.token, method, a, args.clip))

    return 0


if __name__ == "__main__":
    sys.exit(main())
