"""真机探针：测量软键盘弹出过程的帧率与输入框 DOM 定位。

2026-11-05：用户反馈「输入法向上的过程有迟滞感」。要定位原因，需要客观数据：
  1. 键盘动画期间 WebView 的 **rAF 帧间隔**（掉帧就是迟滞的物理来源）；
  2. 输入框的 **祖先链**（找出「哪个祖先是 fixed 容器」）——
     决定 transform 补偿该挂在哪一层。

判据：
  - 平时帧间隔 ~11ms（90fps）；
  - 键盘动画期间若出现连续 >32ms 的长帧 -> 掉帧，根因是
    视口连续变化导致 WebView 每帧全量重排。

用法：
    python scripts/probe_keyboard.py
"""
from __future__ import annotations

import json
import os
import re
import subprocess
import threading
import time
from http.server import BaseHTTPRequestHandler, HTTPServer

ADB = r"D:/programs/android-sdk/platform-tools/adb.exe"
PROBE = r"E:/develop/dsh-workspace/qianxun/.tmp/probe"
SHOTS = r"E:/develop/dsh-workspace/qianxun/.tmp/shots"
PKG = "com.qianxun.qianxun_mobile"
ACT = ".MainActivity"
PROBE_URL = "http://192.168.20.2:8898/probe"
# 输入框中心坐标的兜底值（实测稳定；uiautomator 偶尔吐不出 WebView 的
# a11y 节点，那时用这个兜底）。
FALLBACK_INPUT_XY = (609, 2406)

# ── 探针接收端（内嵌，跑完自动关闭）───────────────────────────────
_received: list[dict] = []


class _ProbeHandler(BaseHTTPRequestHandler):
    def do_POST(self):  # noqa: N802
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length).decode("utf-8", "replace")
        try:
            _received.append(json.loads(raw))
        except Exception:
            pass
        self.send_response(204)
        self.send_header("Access-Control-Allow-Origin", "*")
        self.end_headers()

    def do_OPTIONS(self):  # noqa: N802
        self.send_response(204)
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Headers", "*")
        self.end_headers()

    def log_message(self, *args):  # 静音
        return


def start_probe_server(port: int = 8898) -> HTTPServer:
    server = HTTPServer(("0.0.0.0", port), _ProbeHandler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def sh(*args: str, timeout: int = 60) -> str:
    proc = subprocess.run(
        [ADB, "shell", *args],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=timeout,
    )
    return proc.stdout or ""


def host(*args: str, timeout: int = 60) -> str:
    proc = subprocess.run(
        [ADB, *args],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=timeout,
    )
    return proc.stdout or ""


def top_package() -> str:
    out = sh("dumpsys", "activity", "activities")
    m = re.search(r"topResumedActivity=ActivityRecord\{\S+ \S+ (\S+)/", out)
    return m.group(1) if m else "?"


def dump(tag: str) -> str:
    os.makedirs(PROBE, exist_ok=True)
    sh("uiautomator", "dump", f"/sdcard/{tag}.xml")
    host("pull", f"/sdcard/{tag}.xml", os.path.join(PROBE, f"{tag}.xml"))
    with open(os.path.join(PROBE, f"{tag}.xml"), encoding="utf-8", errors="replace") as fh:
        return fh.read()


def close_preview() -> bool:
    """关掉 DSH 的图片预览层（全屏覆盖容器，会吞掉所有触摸）。"""
    xml = dump("kb_probe1")
    if "图片预览：" not in xml:
        return False
    for m in re.finditer(r"<node[^>]*>", xml):
        tag = m.group(0)
        if 'clickable="true"' not in tag:
            continue
        desc = re.search(r'content-desc="([^"]*)"', tag)
        text = re.search(r'text="([^"]*)"', tag)
        label = (desc.group(1) if desc else "") or (text.group(1) if text else "")
        if label.strip() == "关闭" or label.strip().endswith(" 关闭"):
            b = re.search(r'bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', tag)
            if not b:
                continue
            g = tuple(int(x) for x in b.groups())
            sh("input", "tap", str(g[2] - 16), str((g[1] + g[3]) // 2))
            time.sleep(2)
            if "图片预览：" not in dump("kb_probe2"):
                return True
    return False


def find_edit_text() -> tuple[int, int] | None:
    """找可见的输入框。

    ⚠️ 不能按绝对 y 位置筛：键盘弹起时输入框会随视口上移（实测 y 从
    2341 变到约 1000）。改为按**高度**排除零高度的占位节点
    （实测有一个 bottom=336 的零高 EditText），再取最下方的那个。
    """
    xml = dump("kb_probe3")
    best = None
    for m in re.finditer(r"<node[^>]*>", xml):
        tag = m.group(0)
        if "EditText" not in tag:
            continue
        b = re.search(r'bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', tag)
        if not b:
            continue
        g = tuple(int(x) for x in b.groups())
        if g[3] - g[1] < 20:  # 零高度的占位节点
            continue
        if best is None or g[3] > best[3]:
            best = g
    if not best:
        # uiautomator 偶尔吐不出 WebView 的 a11y 节点 -> 用兜底坐标
        return FALLBACK_INPUT_XY
    return ((best[0] + best[2]) // 2, (best[1] + best[3]) // 2)


def main() -> int:
    server = start_probe_server()
    print("探针接收端已起（0.0.0.0:8898）")

    # 1) 切到 Flutter 端并校验前台
    sh("am", "force-stop", "com.qianxun.mobile")
    sh("am", "start", "-n", f"{PKG}/{ACT}")
    for _ in range(30):
        time.sleep(1)
        if PKG in top_package():
            break
    print("前台:", top_package())
    time.sleep(3)

    # 2) 深链进 DSH 页（两个 App 注册了同一个 scheme，深链可能唤起另一个，
    #    所以进页后再校验一次前台）
    sh("am", "start", "-a", "android.intent.action.VIEW", "-d", "qianxun://app/dsh")
    time.sleep(14)
    print("深链后前台:", top_package())
    if PKG not in top_package():
        print("⚠️ 深链唤起了别的 App，改用连接页点击")
        sh("am", "start", "-n", f"{PKG}/{ACT}")
        time.sleep(5)
        sh("input", "tap", "400", "560")
        time.sleep(10)
        print("点击后前台:", top_package())

    # 3) 关掉预览层（否则全屏拦截触摸）
    print("预览层已关:", close_preview())

    # 3.5) **重启 App** 来保证键盘是收起的。
    # ⚠️ 不能用 KEYCODE_BACK：DSH 页面会拦截 BACK（自己的返回逻辑），
    #    键盘收不起来 -> 后面 tap 输入框只是切换焦点，键盘不弹，
    #    测出来就是「vv 恒定不变」的假象。
    sh("am", "force-stop", PKG)
    time.sleep(1.5)
    sh("am", "start", "-n", f"{PKG}/{ACT}")
    for _ in range(20):
        time.sleep(1)
        if PKG in top_package():
            break
    # 深链 + 前台校验重试（钉钉等第三方偶尔会弹到前台）
    for attempt in range(3):
        sh("am", "start", "-a", "android.intent.action.VIEW", "-d", "qianxun://app/dsh")
        time.sleep(14)
        top = top_package()
        print(f"重启后前台(尝试 {attempt + 1}):", top)
        if PKG in top:
            break
        sh("am", "start", "-n", f"{PKG}/{ACT}")
        time.sleep(5)
    if PKG not in top_package():
        print("前台始终不是目标 App，放弃")
        server.shutdown()
        return 5

    # 4) 找输入框并点击
    pos = find_edit_text()
    print("输入框坐标:", pos)
    if not pos:
        shot = os.path.join(SHOTS, "kb_probe_nofind.png")
        os.makedirs(SHOTS, exist_ok=True)
        with open(shot, "wb") as fh:
            fh.write(
                subprocess.run([ADB, "exec-out", "screencap", "-p"], capture_output=True, timeout=60).stdout
            )
        print("未找到输入框，已截图:", shot)
        return 2
    _received.clear()
    sh("input", "tap", str(pos[0]), str(pos[1]))
    time.sleep(0.8)
    snap_path = os.path.join(SHOTS, "kb_show_mid.png")
    with open(snap_path, "wb") as fh:
        fh.write(subprocess.run([ADB, "exec-out", "screencap", "-p"], capture_output=True, timeout=60).stdout)
    print("show 中途截图:", snap_path)
    time.sleep(1.8)  # 等 kbshow 采样结束（1.5s）

    # 4.5) 收起键盘，测 hide 动画
    sh("input", "keyevent", "4")
    time.sleep(0.8)
    hide_path = os.path.join(SHOTS, "kb_hide_mid.png")
    with open(hide_path, "wb") as fh:
        fh.write(subprocess.run([ADB, "exec-out", "screencap", "-p"], capture_output=True, timeout=60).stdout)
    print("hide 中途截图:", hide_path)
    time.sleep(1.8)

    # 5) 读探针数据（全部落盘，便于事后分析）
    dump_path = os.path.join(PROBE, "kb_traces.json")
    with open(dump_path, "w", encoding="utf-8") as fh:
        json.dump(_received, fh, ensure_ascii=False, indent=1)
    print("beacon 已落盘:", dump_path, f"({len(_received)} 条)")
    traces = [d for d in _received if d.get("kind") in ("kbshow", "kbhide", "kbsnap")]
    print("trace 条数:", len(traces))
    for d in traces:
        if d.get("kind") == "kbsnap":
            print(f"[kbsnap] kb={d.get('kb')}")
            for i, c in enumerate(d.get("chain") or []):
                print(
                    f'  [{i}] {c["tag"]:6} slot={str(c["slot"])[:30]:30} '
                    f'disp={c["display"]:10} tf={str(c["transform"])[:22]:22} '
                    f'oy={c["oy"]:7} t={c["t"]:5} b={c["b"]:5} h={c["h"]:5} {c["cls"]}'
                )
            continue
        fr = d.get("frames") or []
        if fr:
            long = [(i, f) for i, f in enumerate(fr) if f > 32]
            print(
                f'[{d.get("kind")}] 帧数 {len(fr)}  最长 {max(fr)}ms  '
                f'>32ms 长帧 {len(long)} 个: {long[:12]}'
            )
    server.shutdown()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
