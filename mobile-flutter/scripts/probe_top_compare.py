"""真机探针：同时刻对照两个客户端「对话 | 轨迹」行**上一行**的差异。

2026-11-05：用户反馈 Flutter 端在「对话 / 轨迹」那一行的**上一行**多了一行换行。
之前两次 uiautomator dump 是不同时刻、不同会话状态（RN 端 dump 时页面还是旧会话，
Flutter 端 dump 时已经切到新会话），不能直接比——必须同刻抓。

做法：
1. 依次切到 RN 端 / Flutter 端，各自进入 DSH 页、等渲染、关掉图片预览层；
2. 各自 uiautomator dump，找「对话」节点的 y；
3. 把该 y **以上**的所有节点按 (y, class, label) 列出——这就是「上一行」区域；
4. 两端做差集。

用法：
    python scripts/probe_top_compare.py
"""
from __future__ import annotations

import os
import re
import subprocess
import time

ADB = r"D:/programs/android-sdk/platform-tools/adb.exe"
PROBE = r"E:/develop/dsh-workspace/qianxun/.tmp/probe"
SHOTS = r"E:/develop/dsh-workspace/qianxun/.tmp/shots"

TARGETS = [
    # ⚠️ RN 端的 Activity 类名包前缀是 `com.qianxunmobile`，**不是** `com.qianxun.mobile`，
    # 所以 `.MainActivity` 简写会解析成不存在的 `com.qianxun.mobile.MainActivity`，
    # `am start` 静默失败（不报错、前台不动）——曾导致「两端对照」抓到的是同一个 App。
    # 真值来自 `adb shell cmd package resolve-activity --brief -c android.intent.category.LAUNCHER`。
    ("RN", "com.qianxun.mobile", "com.qianxunmobile.MainActivity"),
    ("Flutter", "com.qianxun.qianxun_mobile", ".MainActivity"),
]


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


def dump(tag: str) -> str:
    os.makedirs(PROBE, exist_ok=True)
    sh("uiautomator", "dump", f"/sdcard/{tag}.xml")
    host("pull", f"/sdcard/{tag}.xml", os.path.join(PROBE, f"{tag}.xml"))
    with open(os.path.join(PROBE, f"{tag}.xml"), encoding="utf-8", errors="replace") as fh:
        return fh.read()


def parse(xml: str) -> list[tuple[int, int, int, str, str]]:
    """-> [(y, x, cls_short, label, bounds)]"""
    out = []
    for m in re.finditer(r"<node[^>]*>", xml):
        tag = m.group(0)
        cls = re.search(r'class="([^"]*)"', tag)
        text = re.search(r'text="([^"]*)"', tag)
        desc = re.search(r'content-desc="([^"]*)"', tag)
        bounds = re.search(r'bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', tag)
        if not bounds:
            continue
        g = tuple(int(x) for x in bounds.groups())
        label = (text.group(1) if text else "") or (desc.group(1) if desc else "")
        label = label.strip().replace("\n", " ")
        if len(label) > 46:
            label = label[:46] + "…"
        out.append((g[1], g[0], (cls.group(1) if cls else "?").split(".")[-1], label, str(g)))
    return out


def shot(name: str) -> str:
    os.makedirs(SHOTS, exist_ok=True)
    path = os.path.join(SHOTS, name)
    proc = subprocess.run(
        [ADB, "exec-out", "screencap", "-p"], capture_output=True, timeout=60
    )
    with open(path, "wb") as fh:
        fh.write(proc.stdout)
    print(f"  shot -> {name} ({len(proc.stdout)} B)")
    return path


def crop(src: str, dst: str, top: int = 180, bottom: int = 820) -> str:
    """裁出头部区域（y=top..bottom），方便并排看「对话|轨迹」上面那几行。"""
    try:
        from PIL import Image  # noqa: PLC0415
    except ImportError:
        print("  （无 PIL，跳过裁剪）")
        return src
    img = Image.open(src)
    img.crop((0, top, img.width, bottom)).save(dst)
    print(f"  crop -> {os.path.basename(dst)} ({img.width}x{bottom - top})")
    return dst


def top_package() -> str:
    out = sh("dumpsys", "activity", "activities")
    m = re.search(r"topResumedActivity=ActivityRecord\{\S+ \S+ (\S+)/", out)
    if m:
        return m.group(1)
    m = re.search(r"mResumedActivity: ActivityRecord\{\S+ \S+ (\S+)/", out)
    return m.group(1) if m else "(未知)"


def enter_dsh(pkg: str, activity: str, others: list[str]) -> None:
    """切到目标 App 并**校验**前台确实是它。

    ⚠️ 踩过两个坑：
    1. 光 `am start` 不够——目标 Activity 若已在栈顶（singleTask），系统不重建，
       前台可能仍是另一个 App，于是「对照」抓到的是同一个 App（当时就发生了：
       以为在抓 RN 端，实际抓的是 Flutter 端，两边数据一模一样）。
    2. `am start -S` 会 force-stop 后重启，但 RN（Expo）冷启动慢，6 秒不够，
       后续 tap 会落到**桌面图标**上（实测把「天气」App 打开了）。
    所以：force-stop 掉其它 App -> 启动 -> **轮询等前台真的切过去** ->
    再点卡片 -> 再次校验。
    """
    for other in others:
        sh("am", "force-stop", other)
    time.sleep(1)
    sh("am", "start", "-n", f"{pkg}/{activity}")
    top = ""
    for _ in range(30):  # 最多等 30 秒
        time.sleep(1)
        top = top_package()
        if pkg in top:
            break
    print(f"  前台 = {top}  (期望 {pkg})")
    if pkg not in top:
        print("  ⚠️ 切换失败，后续数据不可信")
        return
    # 卡片在 y 448..679（uiautomator 实测）
    sh("input", "tap", "400", "560")
    for _ in range(20):
        time.sleep(1)
        top2 = top_package()
        if pkg in top2:
            break
    print(f"  进页后前台 = {top2}")


def close_preview() -> bool:
    """关掉 DSH 的图片预览层（全屏覆盖容器，会吞触摸）。"""
    xml = dump("cmp_probe")
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
            if "图片预览：" not in dump("cmp_probe2"):
                return True
    return False


def collect(tag: str) -> dict:
    xml = dump(tag)
    nodes = parse(xml)
    # 「对话」行的 y —— 「上一行」区域的基准线
    talk_y = None
    for y, x, cls, label, b in nodes:
        if label == "对话" and cls == "View":
            talk_y = y
            break
    if talk_y is None:
        for y, x, cls, label, b in nodes:
            if label == "对话":
                talk_y = y
                break
    above = [n for n in nodes if talk_y is not None and n[0] < talk_y]
    # 只保留「有文字/描述」的可见节点，排除对话正文噪声（正文行数多且内容随会话变）
    visible = [n for n in above if n[3]]
    return {
        "talk_y": talk_y,
        "visible": visible,
        "all": above,
        "webview_y": next((n[0] for n in nodes if n[2] == "WebView"), None),
        "total": len(nodes),
    }


def main() -> int:
    results = {}
    all_pkgs = [pkg for _, pkg, _ in TARGETS]
    for tag, pkg, activity in TARGETS:
        print(f"=== {tag} ({pkg}) ===")
        enter_dsh(pkg, activity, [p for p in all_pkgs if p != pkg])
        closed = close_preview()
        print(f"  预览层已关: {closed}")
        data = collect(f"cmp_{tag}")
        results[tag] = data
        print(f"  「对话」行 y = {data['talk_y']}   WebView y = {data['webview_y']}   节点数 {data['total']}")
        full = shot(f"cmp_{tag}_full.png")
        crop(full, os.path.join(SHOTS, f"cmp_{tag}_head.png"))
        time.sleep(1)

    rn, fl = results["RN"], results["Flutter"]
    print("\n" + "=" * 72)
    print(f"「对话」行 y：RN={rn['talk_y']}  Flutter={fl['talk_y']}")
    print(f"WebView 起点 y：RN={rn['webview_y']}  Flutter={fl['webview_y']}")
    print("=" * 72)

    def sig(n):
        """对比用的签名：(y, class, label) —— 去掉 x，避免两边排版微差干扰。"""
        return (n[0], n[2], n[3])

    rn_map = {}
    for n in rn["visible"]:
        rn_map.setdefault(sig(n), n)
    fl_map = {}
    for n in fl["visible"]:
        fl_map.setdefault(sig(n), n)

    print("\n--- 两端都有（共同基线）---")
    for k in sorted(set(rn_map) & set(fl_map), key=lambda k: (k[0], k[2])):
        print(f"  y={k[0]:5} {k[1]:12} {k[2]}")

    print("\n--- 只有 RN 有 ---")
    for k in sorted(set(rn_map) - set(fl_map), key=lambda k: (k[0], k[2])):
        print(f"  y={k[0]:5} {k[1]:12} {k[2]}")

    print("\n--- 只有 Flutter 有 ---")
    for k in sorted(set(fl_map) - set(rn_map), key=lambda k: (k[0], k[2])):
        print(f"  y={k[0]:5} {k[1]:12} {k[2]}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
