"""用 CDP 的 Network / Runtime 域抓一次完整的页面加载，看请求到底怎么失败的。

为什么需要它
------------
真机上「页面已加载但白屏、custom.js 未生效」的排查卡住了：
  · eval 能看到页面状态，但**看不到它为什么这样**
  · 手机 shell 只有 toybox nc，nc 的输出被 shell 变量吞掉，取不到响应头
  · 真机 RTT 230–600ms，任何「发个 fetch 等它回来」的探针都可能自己先超时

Network.loadingFailed 会直接给出 `net::ERR_*` 或 canceledText，
Runtime.exceptionThrown 给出 JS 异常 —— 这两个是定性的关键，
比猜十条都准。

用法：python scripts/cdp_trace.py [秒数]
"""
from __future__ import annotations

import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import cdp_webview as C  # noqa: E402


def main() -> int:
    seconds = float(sys.argv[1]) if len(sys.argv) > 1 else 25.0
    picked = C.pick_socket("192.168.")
    if picked is None:
        return 1
    _sock, port = picked
    target = next((p for p in C.pages(port) if p.get("type") == "page"), None)
    ws = C.WS(C.ws_url(port, target))

    for domain in ("Network.enable", "Runtime.enable", "Log.enable", "Page.enable"):
        ws.call(domain)

    ws.call("Page.reload", {"ignoreCache": True})

    reqs: dict[str, dict] = {}
    failed: list[str] = []
    console: list[str] = []
    deadline = time.time() + seconds
    ws.sock.settimeout(2.0)
    while time.time() < deadline:
        try:
            msg = json.loads(ws.recv())
        except Exception:
            continue
        m = msg.get("method", "")
        p = msg.get("params", {}) or {}
        if m == "Network.requestWillBeSent":
            r = p.get("request", {})
            reqs[r.get("id")] = {
                "url": r.get("url", "").split("/")[-1][:46],
                "t": round(time.time() - (deadline - seconds), 2),
            }
        elif m == "Network.responseReceived":
            r = p.get("response", {})
            rid = p.get("requestId")
            if rid in reqs:
                reqs[rid]["status"] = r.get("status")
                reqs[rid]["mime"] = (r.get("mimeType") or "")[:28]
                reqs[rid]["proto"] = r.get("protocol")
        elif m == "Network.loadingFailed":
            rid = p.get("requestId")
            url = reqs.get(rid, {}).get("url", rid)
            failed.append(
                f"{url:<48} {p.get('errorText')} "
                f"canceled={p.get('canceledText', '-')} type={p.get('type')}"
            )
        elif m == "Network.loadingFinished":
            rid = p.get("requestId")
            if rid in reqs:
                reqs[rid]["ok"] = True
        elif m == "Runtime.consoleAPICalled":
            args = " ".join(str(a.get("value", ""))[:90] for a in p.get("args", []))
            if args.strip():
                console.append(f"[{p.get('type')}] {args}")
        elif m == "Runtime.exceptionThrown":
            d = p.get("exceptionDetails", {})
            txt = (d.get("exception", {}) or {}).get("description") or d.get("text", "")
            console.append(f"[EXCEPTION] {str(txt)[:200]}")
        elif m == "Log.entryAdded":
            e = p.get("entry", {})
            console.append(f"[log/{e.get('level')}] {str(e.get('text'))[:180]}")

    print(f"=== 请求 {len(reqs)} 个 ===")
    for v in sorted(reqs.values(), key=lambda x: x["t"]):
        mark = "✓" if v.get("ok") else (" " if "status" not in v else "✗")
        print(f" {mark} {v['t']:>5}s  {v['url']:<46} {v.get('status', '—')} "
              f"{v.get('proto', '—')} {v.get('mime', '')}")

    print(f"\n=== 失败 {len(failed)} 个 ===")
    for f in failed:
        print("  ✗", f)

    print(f"\n=== 控制台 {len(console)} 条 ===")
    for c in console[-30:]:
        print("  ", c)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
