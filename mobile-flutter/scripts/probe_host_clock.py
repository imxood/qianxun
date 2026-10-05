#!/usr/bin/env python3
"""检验宿主 `updatedAt` 是否可用于「多久以前」的展示。

争议
----
设计提案 B 声称宿主时钟比真实时间慢约 9h55m
（`docs/design-proposals/B-state-model.md`）。若属实，我们现在渲染的
「22 小时前」全是错的 —— 而**相对时间是这个产品最主要的一手信息**
（05 §4.2 元信息行就有它）。

## 决定性检验
------------
单条会话的 `updatedAt` 与当前时间相差大，**不能证明时钟偏移** —— 那只
说明那条会话很久没被动过。

真正的判据是**全表 `updatedAt` 的最大值**：用户此刻正在跑 agent（本会话
就是），所以至少有一条会话必须是**几分钟内**更新过的。如果 max 仍然落后
好几小时，那才是时钟问题。

同时对比三个时间源：
  1. 本机（跑探针的这台）
  2. 宿主应用所在机器 —— 这里假设与本机同机
  3. 设备的 `Date`
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import subprocess
import urllib.error
import urllib.request


def call(host: str, port: int, token: str) -> dict:
    body = json.dumps({
        "type": "client-request",
        "rpcId": "clock-probe",
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


def iso(ms: int) -> str:
    return dt.datetime.fromtimestamp(ms / 1000, dt.timezone.utc).isoformat()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--token", required=True)
    ap.add_argument("--host", default="192.168.20.2")
    ap.add_argument("--port", type=int, default=23090)
    ap.add_argument("--serial", default="", help="同时读设备时间")
    args = ap.parse_args()

    now = dt.datetime.now(dt.timezone.utc)
    print(f"本机现在 (UTC) : {now.isoformat()}\n")

    res = call(args.host, args.port, args.token)
    if "_err" in res or "_http" in res:
        print(json.dumps(res, ensure_ascii=False, indent=2))
        return 1

    items = res.get("result", {}).get("value", {}).get("items", [])
    stamps = [it.get("updatedAt") for it in items if isinstance(it.get("updatedAt"), int)]
    if not stamps:
        print("没有任何 updatedAt")
        return 1

    newest, oldest = max(stamps), min(stamps)
    newest_dt = dt.datetime.fromtimestamp(newest / 1000, dt.timezone.utc)
    lag = now - newest_dt

    print(f"会话数            : {len(stamps)}")
    print(f"最新 updatedAt    : {iso(newest)}")
    print(f"最旧 updatedAt    : {iso(oldest)}")
    print(f"最新那条距今      : {lag}")
    print()

    if args.serial:
        try:
            r = subprocess.run(
                ["adb", "-s", args.serial, "shell", "date", "-u", "+%Y-%m-%dT%H:%M:%SZ"],
                capture_output=True, text=True, timeout=20,
            )
            print(f"设备时间 (UTC)   : {r.stdout.strip()}")
        except Exception as e:  # noqa: BLE001
            print(f"设备时间读取失败  : {e}")
        print()

    verdict = (
        "✅ 宿主时钟可用：最新会话几分钟内更新过，相对时间可信"
        if lag < dt.timedelta(hours=2) else
        "⚠️ 存疑：最新会话也落后 2 小时以上，但可能只是没人在跑 agent"
        if lag < dt.timedelta(hours=12) else
        "❌ 强烈指向时钟偏移：此刻有人在跑 agent，最新会话却落后很久"
    )
    print(verdict)
    print()
    print("注意：即使判为偏移，也要先排除「这段时间真的没人跑 agent」。")
    print("最可靠的下一步：在网关侧主动 touch 一个会话，再看 updatedAt 是否跟着走。")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
