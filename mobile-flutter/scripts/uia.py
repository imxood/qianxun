"""真机 UI 操作的统一封装（uiautomator2 + adb + CDP）。

为什么要有这一层
----------------
迭代要在这台真机上反复跑，而「点一下」「截个图」「读层级」「看页面」
散在四个工具里，每轮都得手拼一长串命令，几十轮之后一定会出错。
这里收成几个函数，行为固定、失败会抛，而不是静默返回半截结果。

⚠️ 真机上同时装着 **两个 WebView**（iot-cloud 的 VPN 应用 + 我们的 DSH），
   而且调试 socket 只有后者的名字带 `webview_devtools_remote_<pid>`。
   实测过一次：按名字排序取第一个会连到 VPN 应用上去，
   看到的是 `http://tauri.localhost/`，跟本项目毫无关系 ——
   所以下面按**页面 URL** 筛，不要按 socket 名。

用法：
  python scripts/uia.py dump                 # 打印 UI 层级摘要
  python scripts/uia.py shot .tmp/x.png      # 截图（二进制安全）
  python scripts/uia.py tap-text "maxu-pc-msi"
  python scripts/uia.py tap 600 1200
  python scripts/uia.py log                  # 抓 DSH 相关日志
  python scripts/uia.py web                   # 连我们自己的 WebView，探页面状态
"""
from __future__ import annotations

import json
import subprocess
import sys
import time
from pathlib import Path

import uiautomator2 as u2

ROOT = Path(__file__).resolve().parent.parent
ADB = Path(r"D:\programs\android-sdk\platform-tools\adb.exe")
CDP = ROOT / "scripts" / "cdp_webview.py"
PKG = "com.qianxun.qianxun_mobile"
SERIAL = "AEQC8P55MFCYBUFA"


def d() -> u2.Device:
    return u2.connect(SERIAL)


def adb(*args: str, timeout: int = 120) -> bytes:
    return subprocess.run(
        [str(ADB), "-s", SERIAL, *args],
        capture_output=True, timeout=timeout,
    ).stdout


def load_tree(dev) -> dict:
    """uiautomator2 的 dump_hierarchy 有时给 dict、有时给 XML 字符串。

    ⚠️ 实测踩过：直接喂给 summarize 会报
       `'str' object has no attribute 'get'`。两种都兜住。
    """
    tree = dev.dump_hierarchy()
    if isinstance(tree, dict):
        return tree
    import xml.etree.ElementTree as ET
    return {
        "className": "root", "text": "", "contentDescription": "",
        "resourceId": "", "clickable": False, "bounds": {},
        "children": [
            {
                "className": n.get("class", ""),
                "text": n.get("text", ""),
                "contentDescription": n.get("content-desc", ""),
                "resourceId": n.get("resource-id", ""),
                "clickable": n.get("clickable") == "true",
                "bounds": box(n.get("bounds", "")),
                "children": [
                    {
                        "className": c.get("class", ""),
                        "text": c.get("text", ""),
                        "contentDescription": c.get("content-desc", ""),
                        "resourceId": c.get("resource-id", ""),
                        "clickable": c.get("clickable") == "true",
                        "bounds": box(c.get("bounds", "")),
                        "children": [],
                    }
                    for c in n
                ],
            }
            for n in ET.fromstring(tree)
        ],
    }


def box(spec: str) -> dict:
    try:
        l, t, r, b = (int(v) for v in spec.replace("][", ",").strip("[]").split(","))
        return {"left": l, "top": t, "right": r, "bottom": b}
    except Exception:
        return {}


def summarize(node: dict, depth: int = 0, out: list[str] | None = None) -> list[str]:
    """把 uiautomator2 的层级压成一行一个可点的元素 —— 层级原样 dump 太长。"""
    if out is None:
        out = []
    cls = node.get("className", "")
    text = (node.get("text") or "").strip()
    desc = (node.get("contentDescription") or "").strip()
    rid = node.get("resourceId") or ""
    clickable = node.get("clickable")
    if clickable or text or desc:
        b = node.get("bounds", {})
        out.append(
            f"{'  ' * depth}{cls.split('.')[-1]:<14} "
            f"click={str(clickable):<5} "
            f"box={b.get('left')},{b.get('top')}-{b.get('right')},{b.get('bottom')} "
            f"{(('text=' + repr(text)) if text else ''):<34}"
            f"{(('desc=' + repr(desc)) if desc else ''):<24}"
            f"{(('id=' + rid) if rid else '')}"
        )
    for c in node.get("children", []) or []:
        summarize(c, depth + 1, out)
    return out


def main() -> int:
    action = sys.argv[1] if len(sys.argv) > 1 else "dump"
    dev = d()

    if action == "dump":
        lines = summarize(load_tree(dev))
        print("\n".join(lines) if lines else "(空)")
        return 0

    if action == "shot":
        path = Path(sys.argv[2] if len(sys.argv) > 2 else ".tmp/phone.png")
        path.parent.mkdir(parents=True, exist_ok=True)
        # adb 的二进制输出不能走 PowerShell 的 > 重定向，会被当文本处理
        path.write_bytes(adb("exec-out", "screencap", "-p"))
        print(f"{path}  {path.stat().st_size} 字节")
        return 0

    if action == "tap-text":
        want = sys.argv[2]
        hit = None
        for line in summarize(load_tree(dev)):
            if want in line:
                hit = line
                break
        if not hit:
            print(f"没找到含 {want!r} 的元素")
            return 1
        box = hit.split("box=")[1].split()[0]
        x1, y1, x2, y2 = (int(v) for v in box.split(","))
        dev.click((x1 + x2) // 2, (y1 + y2) // 2)
        print("点了", hit.strip()[:90])
        return 0

    if action == "tap":
        dev.click(int(sys.argv[2]), int(sys.argv[3]))
        print("点了", sys.argv[2], sys.argv[3])
        return 0

    if action == "log":
        raw = adb("logcat", "-d", "-t", str(sys.argv[2] if len(sys.argv) > 2 else 400))
        text = raw.decode("utf-8", "ignore")
        keep = [ln for ln in text.splitlines()
                if any(k in ln for k in ("flutter", "qx", "dsh", "DshPage", "WebView",
                                         "chromium", "qianxun_mobile"))]
        print("\n".join(keep[-60:]) or "(没有相关日志)")
        return 0

    if action == "clear-log":
        adb("logcat", "-c")
        print("已清空 logcat")
        return 0

    if action == "restart":
        adb("shell", "am", "force-stop", PKG)
        time.sleep(1.5)
        adb("shell", "am", "start", "-n", f"{PKG}/.MainActivity")
        print("已重启")
        return 0

    if action == "web":
        expr = sys.argv[2] if len(sys.argv) > 2 else "document.title"
        proc = subprocess.run(
            [sys.executable, str(CDP), "--serial", SERIAL, "eval", expr],
            capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=180,
        )
        print(proc.stdout.strip())
        if proc.stderr.strip():
            print(proc.stderr.strip()[:400])
        return 0

    print(__doc__)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
