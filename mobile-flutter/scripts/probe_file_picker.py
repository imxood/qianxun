"""真机探针：关掉 DSH 图片预览层 -> 点「+」-> 找「文件」-> 判定是否弹出系统选择器。

2026-11-05：验证 Flutter 端（versionCode 2009 旧包）的文件选择器行为。
判定依据 = **是否启动了新的 Activity**（系统文件选择器是独立 Activity），
比看截图可靠。

用法：
    python scripts/probe_file_picker.py [--package <pkg>] [--label <名字>]
"""
from __future__ import annotations

import argparse
import os
import re
import subprocess
import time

ADB = r"D:/programs/android-sdk/platform-tools/adb.exe"
SHOTS = r"E:/develop/dsh-workspace/qianxun/.tmp/shots"
PROBE = r"E:/develop/dsh-workspace/qianxun/.tmp/probe"


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


def shot(name: str) -> None:
    os.makedirs(SHOTS, exist_ok=True)
    path = os.path.join(SHOTS, name)
    proc = subprocess.run(
        [ADB, "exec-out", "screencap", "-p"], capture_output=True, timeout=60
    )
    with open(path, "wb") as fh:
        fh.write(proc.stdout)
    print(f"  shot -> {name} ({len(proc.stdout)} B)")


def dump(tag: str) -> str:
    os.makedirs(PROBE, exist_ok=True)
    sh("uiautomator", "dump", f"/sdcard/{tag}.xml")
    host("pull", f"/sdcard/{tag}.xml", os.path.join(PROBE, f"{tag}.xml"))
    with open(os.path.join(PROBE, f"{tag}.xml"), encoding="utf-8", errors="replace") as fh:
        return fh.read()


def top_activity() -> str:
    out = sh("dumpsys", "activity", "activities")
    m = re.search(r"mResumedActivity[^\n]*", out)
    if m:
        return m.group(0).strip()[:150]
    m = re.search(r"topResumedActivity[^\n]*", out)
    return m.group(0).strip()[:150] if m else "(未找到)"


def nodes_with(xml: str, keyword: str) -> list[tuple[str, tuple[int, int, int, int]]]:
    out = []
    for m in re.finditer(r"<node[^>]*>", xml):
        tag = m.group(0)
        text = re.search(r'text="([^"]*)"', tag)
        desc = re.search(r'content-desc="([^"]*)"', tag)
        label = (text.group(1) if text else "") or (desc.group(1) if desc else "")
        if keyword in label and len(label) < 50:
            b = re.search(r'bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', tag)
            if b:
                out.append((label, tuple(int(x) for x in b.groups())))
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--package", default="com.qianxun.qianxun_mobile")
    ap.add_argument("--label", default="flutter")
    args = ap.parse_args()

    print("=== 0. 起点 ===")
    print("  top activity:", top_activity())

    # 1) 关掉 DSH 图片预览层（它是一个全屏覆盖容器，会吞掉所有触摸）
    print("\n=== 1. 关闭图片预览层 ===")
    xml = dump("probe_p1")
    preview = nodes_with(xml, "图片预览：")
    if preview:
        print(f"  预览层仍在: {preview[0]}")
    for label, (x1, y1, x2, y2) in nodes_with(xml, "关闭"):
        print(f"  候选关闭按钮: {label!r} {((x1,y1,x2,y2))}")
    # tab 上的 X 在预览层之上（y 较小），逐个试
    clicked = False
    for label, (x1, y1, x2, y2) in nodes_with(xml, "关闭"):
        # X 图标位于文字右侧
        cx = x2 - 16
        cy = (y1 + y2) // 2
        print(f"  点 {label!r} 的 X 候选 ({cx},{cy})")
        sh("input", "tap", str(cx), str(cy))
        time.sleep(2)
        xml2 = dump("probe_p1b")
        if not nodes_with(xml2, "图片预览："):
            print("  ✅ 预览层已关闭")
            clicked = True
            break
    if not clicked:
        print("  ⚠️ 未能关闭预览层，后续点击可能仍被拦截")

    # 2) 点「+」（添加文件或调用指令）
    print("\n=== 2. 点「+」 ===")
    xml = dump("probe_p2")
    plus = nodes_with(xml, "添加文件或调用指令")
    if not plus:
        print("  ⚠️ 找不到「+」按钮")
        return 2
    (x1, y1, x2, y2) = plus[0][1]
    print(f"  tap + at ({x1},{y1})")
    sh("input", "tap", str(x1 + 12), str(y1 + 12))
    time.sleep(2)
    shot(f"probe_{args.label}_plus.png")

    # 3) 找菜单里的「文件」
    xml = dump("probe_p3")
    file_nodes = [
        (label, b)
        for label, b in nodes_with(xml, "文件")
        if "资源管理器" not in label and "打开" not in label
    ]
    print(f"  菜单里含「文件」的节点: {[n[0] for n in file_nodes]}")
    if not file_nodes:
        print("  ⚠️ 菜单里没有「文件」项")
        shot(f"probe_{args.label}_nomenu.png")
        return 3

    # 4) 点「文件」，判定是否启动新 Activity（系统选择器）
    print("\n=== 3. 点「文件」并判定弹框 ===")
    before = top_activity()
    print("  点击前:", before)
    (x1, y1, x2, y2) = file_nodes[0][1]
    cx, cy = (x1 + x2) // 2, (y1 + y2) // 2
    print(f"  tap 文件 at ({cx},{cy})")
    sh("input", "tap", str(cx), str(cy))
    time.sleep(4)
    after = top_activity()
    print("  点击后:", after)
    shot(f"probe_{args.label}_file.png")
    changed = before != after
    print(f"\n  >>> {'✅ 弹出了新 Activity（系统选择器）' if changed else '❌ 没有新 Activity -> 未弹框'}")
    return 0 if changed else 4


if __name__ == "__main__":
    raise SystemExit(main())
