"""对比度闸门：**实测**定制层自有元素在亮/暗两套主题下的对比度。

为什么不用静态检查
----------------
`scripts/check_contrast.py` 查的是 Flutter 的设计令牌（纯声明式）。
而定制层的颜色是**渲染出来才知道对不对**的：

- `.qx-status-btn` 的底色是 `rgb(127 127 127 / .12)` —— 半透明，
  实际对比度取决于**它下面压着什么**（composer 卡片 / 页面底色）。
  静态算只能拿底色去比，量到的不是用户看到的。
- 弹层的背景、`.qx-hint-*` 挂在 DSH 自己的行里，背景是继承来的。

所以这里走 CDP 进页面，**量真实的 computed color**，
再顺着祖先把半透明层合成出真实底色，按 WCAG 判阈值。

🔴 **覆盖率是硬要求**
--------------------
第 6 版第一版闸门跑出来是「✓ 全部达标」，但 7 项里有 3 项写着
「当前未渲染，跳过」—— 那三项（两个侧栏提示 + 弹层正文）
根本没被量过。

> **「没量到」不等于「没问题」。** 静默跳过会让闸门
> 在元素根本没渲染时也报绿 —— 与 docs/design/15 §8 记的
> 「绿得像没这回事」是同一类错误。

所以现在：
1. 闸门**自己**把元素驱动到会渲染的状态（开弹层、开工作区页）；
2. 任何一个目标在整轮里**一次都没量到**，直接判失败。

用法
----
  python scripts/check_qx_contrast.py                  # 默认跑模拟器
  python scripts/check_qx_contrast.py --serial AEQC…    # 真机（需开 WebView 调试）
  python scripts/check_qx_contrast.py --keep            # 结束后不切回亮色

退出码：有任一项不达标、或有目标没量到 → 1。
"""
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CDP = ROOT / "scripts" / "cdp_webview.py"
PROBE = ROOT / "scripts" / "probe_contrast.js"
ADB = r"D:\programs\android-sdk\platform-tools\adb.exe"
EMU = "emulator-5554"

# ⚠️ **禁止操作真机**（用户本阶段要求只用模拟器）。
#    这个脚本自己会 `cmd uimode night yes/no`，必须确保它切的是模拟器。
FORBIDDEN_SERIALS = ("AEQC8P55MFCYBUFA",)

# 要检查的自有元素：(说明, 选择器)
# ⚠️ `.qx-hint-ctx` 已从列表移除（上下文占用改由 composer 工具栏的
#    「N%」标签承担），故不在目标清单里。
TARGETS = [
    ("状态药丸数值", ".qx-status-val"),
    ("稀疏行提示", ".qx-hint-sparse"),
    ("弹层标题", ".qx-sheet-title"),
    ("弹层关闭键", ".qx-sheet-x"),
    ("弹层正文", ".qx-sheet-body"),
    ("统计行文字", ".qx-stat-text"),
]

OPEN_SHEET = ("(() => { const b = document.querySelector('.qx-status-btn');"
              " if (b) b.click(); return b ? 'ok' : 'no-btn'; })()")
CLOSE_SHEET = "document.querySelectorAll('[data-qx-statussheet]').forEach(n => n.remove()); 1"
OPEN_WS = ("(() => { const f = document.querySelector('[data-slot=\"root\"] > *');"
           " if (f && f.hasAttribute('data-sidebar-collapsed')) {"
           "   const b = document.querySelector('.qx-menu-btn');"
           "   if (b) { b.click(); return 'ok'; } } return 'already'; })()")
CLOSE_WS = ("(() => { const w = ['收起侧边栏','collapse sidebar','close sidebar'];"
            " const bs = document.querySelectorAll('[data-slot=\"sidebar\"] button,"
            " [data-slot=\"sidebar\"] [role=\"button\"]');"
            " for (const b of bs) { const l = (b.getAttribute('aria-label')||'').toLowerCase();"
            "   for (const k of w) if (l.indexOf(k) !== -1) { b.click(); return 'closed'; } }"
            " return 'not-found'; })()")


def cdp(args: list[str], serial: str | None, timeout: int = 180) -> str:
    cmd = [sys.executable, str(CDP)] + (["--serial", serial] if serial else []) + args
    p = subprocess.run(cmd, capture_output=True, text=True,
                       encoding="utf-8", errors="replace", timeout=timeout)
    return p.stdout


def js(expr: str, serial: str | None) -> str:
    out = cdp(["eval", expr], serial)
    lines = [x for x in out.splitlines() if x.strip() and not x.startswith(("设备", "webview", "已连上", "页面", "  page"))]
    return lines[-1].strip() if lines else ""


def measure(serial: str | None) -> dict[str, dict]:
    out = cdp(["eval", "--file", str(PROBE)], serial)
    lines = [x for x in out.splitlines() if x.startswith("{")]
    if not lines:
        return {}
    data = json.loads(lines[-1])
    return {it["name"]: it for it in data.get("items", [])}


# 读 DSH 会话行**自己的**背景色（`.qx-hint-*` 就画在它上面）。
# 合成半透明层：沿祖先往上，第一个不透明的就是行本身的底色。
READ_ROW_BG = r"""(() => {
  const side = document.querySelector('[data-slot="sidebar"]');
  if (!side) return '';
  const row = side.querySelector('[data-row-key]');
  if (!row) return '';
  const P = (c) => {
    const m = /rgba?\(([^)]+)\)/.exec(c);
    if (!m) return null;
    const p = m[1].split(',').map((x) => parseFloat(x));
    return { r: p[0], g: p[1], b: p[2], a: p.length > 3 ? p[3] : 1 };
  };
  const over = (f, b) => ({
    r: f.r * f.a + b.r * (1 - f.a),
    g: f.g * f.a + b.g * (1 - f.a),
    b: f.b * f.a + b.b * (1 - f.a), a: 1,
  });
  let n = row, acc = null;
  while (n && n !== document.documentElement) {
    const c = P(getComputedStyle(n).backgroundColor);
    if (c && c.a > 0) { acc = acc ? over(acc, c) : c; if (c.a >= 1) break; }
    n = n.parentElement;
  }
  if (!acc) return '';
  return 'rgb(' + [acc.r, acc.g, acc.b].map((v) => Math.round(v)).join(',') + ')';
})()"""


def read_row_bg(serial: str | None) -> str:
    out = cdp(["eval", READ_ROW_BG], serial)
    for x in reversed(out.splitlines()):
        x = x.strip()
        if x.startswith("rgb("):
            return x
    return ""


def night(on: bool, serial: str | None) -> bool:
    """切系统深色模式。只在模拟器上自动切；真机需要手动切。"""
    if serial:
        return False
    try:
        subprocess.run([ADB, "-s", EMU, "shell", "cmd uimode night " + ("yes" if on else "no")],
                       capture_output=True, timeout=60)
        return True
    except Exception:
        return False


def _guard_phone(serial: str | None) -> None:
    if serial in FORBIDDEN_SERIALS:
        raise SystemExit(
            "⛔ 拒绝操作真机 " + str(serial)
            + "\n   用户要求本阶段只使用模拟器（" + EMU + "）。")


def collect(serial: str | None, theme: str, seen: dict[str, list],
             dsh_bg: dict[str, tuple[float, float, float]]) -> None:
    """跑一轮：会话页 → 开弹层 → 工作区页，逐个状态量。

    ⚠️ DSH 行背景**必须在各自主题下当场读**。
    第一版把它放到最后统一读，结果深浅两轮都读到亮色的值 ——
    于是「深色 #9aa7b4 压在亮底上」被判 2.35:1，
    看起来像我们的配色坏了，其实是我把两套主题的背景搞混了。
    **闸门自己出 bug 时，报出来的「产品问题」也是假的。**
    """
    states = [
        ("会话页", lambda: None),
        ("弹层", lambda: js(OPEN_SHEET, serial)),
        ("工作区页", lambda: js(OPEN_WS, serial)),
    ]
    for state_name, enter in states:
        if state_name == "弹层":
            enter()
            time.sleep(1.2)
        elif state_name == "工作区页":
            js(CLOSE_SHEET, serial)
            js(CLOSE_WS, serial)          # 先确保收起，再由 OPEN_WS 打开
            enter()
            time.sleep(1.8)
        data = measure(serial)
        for name, it in data.items():
            if not it.get("found"):
                continue
            it = dict(it)
            it["state"] = state_name
            seen.setdefault(name, []).append(it)
        js(CLOSE_SHEET, serial)
        if state_name == "工作区页":
            raw = read_row_bg(serial)
            if raw:
                dsh_bg[theme] = _parse(raw)
            js(CLOSE_WS, serial)
            time.sleep(0.8)


def report(seen: dict[str, list]) -> tuple[int, list[str]]:
    bad = 0
    missing: list[str] = []
    print(f"\n{'元素':<14}{'状态':<10}{'前景':<24}{'背景':<20}{'字号':>7}{'比值':>9}  判定")
    print("─" * 104)
    for name, _sel in TARGETS:
        runs = seen.get(name)
        if not runs:
            missing.append(name)
            print(f"  {name:<12} ⚠ **整轮都没量到**")
            continue
        for it in runs:
            ok = it["pass"]
            if not ok:
                bad += 1
            print(f"  {name:<12}{it['state']:<10}{it['color']:<24}{it['bg']:<20}"
                  f"{it['fontPx']:>6.1f}px {it['ratio']:>8.2f}:1  {'✓' if ok else '✗ 不达标'}")
    return bad, missing


# ── 静态段：我们自己声明的颜色（不透明字面量） ────────────────────────
#
# 为什么前景色可以静态算：我们写在 CSS 里的颜色**全是不透明字面量**，
# 渲染时不会变。**唯一未知的量是「它被画在什么背景上」**：
#   · 自己的不透明底色（sheet / stat-row）→ 静态即可精确算出
#   · DSH 自己的行背景（hint-*）→ 必须实测（见 live 段）
#
# 所以静态段负责「前景 × 已知底色」，实测段只负责**取 DSH 的背景**，
# 两者合起来是精确值，不需要在页面里等那个元素恰好渲染。
STATIC_PAIRS = [
    # (元素, 深色前景, 深色底, 亮色前景, 亮色底, 字号, 是否用实测背景)
    ("弹层正文", "#e8edf2", "#070a0e", "#141a21", "#f1f4f7", 12, False),
    ("统计行底色", "#e8edf2", "#1b1f26", "#141a21", "#f1f4f7", 14, False),
    ("弹层标题", "#f9fafb", "#12161d", "#141a21", "#ffffff", 16, False),
    ("弹层关闭键", "#9aa7b4", "#12161d", "#5a6672", "#ffffff", 18, False),
    ("稀疏行提示", "#8794a0", None, "#5a6672", None, 11, True),
    # 药丸三档。背景是药丸底 rgb(127 127 127 /.14) 压在 composer 卡片上：
    #   深色卡片 ≈ rgb(56,56,57)、亮色 ≈ rgb(237,237,237)（实测）
    # ⚠️ 「低」档在 CSS 里用的是 **DSH 的 --dsw-alias-label-secondary**，
    #    实测 7.81:1（深）/ 4.96:1（亮），由实测段覆盖；
    #    这里记的是我们写的**兜底值**，它也得站得住 ——
    #    「主路径靠宿主、兜底靠自己」，两条都得达标。
    ("药丸-低(兜底)", "#9aa7b4", "#383839", "#61666b", "#ededed", 12, False),
    ("药丸-中", "#ffc24b", "#383839", "#8a5a00", "#ededed", 12, False),
    ("药丸-高", "#ff8a7a", "#383839", "#a3271b", "#ededed", 12, False),
]


def _parse(c: str) -> tuple[float, float, float]:
    c = c.strip()
    if c.startswith("#"):
        c = c[1:]
        if len(c) == 3:
            c = "".join(x * 2 for x in c)
        return (int(c[0:2], 16), int(c[2:4], 16), int(c[4:6], 16))
    m = re.match(r"rgba?\(([^)]+)\)", c)
    p = [float(x) for x in m.group(1).split(",")]
    return (p[0], p[1], p[2])


def _lin(v: float) -> float:
    v /= 255
    return v / 12.92 if v <= 0.03928 else ((v + 0.055) / 1.055) ** 2.4


def _lum(c: tuple[float, float, float]) -> float:
    return 0.2126 * _lin(c[0]) + 0.7152 * _lin(c[1]) + 0.0722 * _lin(c[2])


def _ratio(a: tuple[float, float, float], b: tuple[float, float, float]) -> float:
    la, lb = _lum(a), _lum(b)
    return (max(la, lb) + 0.05) / (min(la, lb) + 0.05)


def static_report(dsh_row_bg: dict[str, tuple[float, float, float]] | None) -> int:
    """用实测到的 DSH 行背景补齐 hint-*，其余纯静态算。"""
    print(f"\n{'静态段（声明色 × 已知底色）':<14}{'主题':<8}{'前景':<12}{'背景':<20}"
          f"{'字号':>7}{'比值':>9}  判定")
    print("─" * 104)
    bad = 0
    for name, dfg, dbg, lfg, lbg, fs, need_dsh in STATIC_PAIRS:
        for theme, fg, bg in (("深色", dfg, dbg), ("亮色", lfg, lbg)):
            if need_dsh:
                if not dsh_row_bg:
                    print(f"  {name:<12}{theme:<8}⚠ 缺 DSH 行背景实测值，跳过")
                    continue
                bgc = dsh_row_bg[theme]
            else:
                bgc = _parse(bg)
            r = _ratio(_parse(fg), bgc)
            need = 3 if (fs >= 24 or (fs >= 18.66 and fs >= 18)) else 4.5
            need = 3 if fs >= 24 else 4.5
            ok = r >= need
            if not ok:
                bad += 1
            bgs = "rgb(%s)" % ",".join(str(int(v)) for v in bgc)
            print(f"  {name:<12}{theme:<8}{fg:<12}{bgs:<20}"
                  f"{fs:>6.0f}px {r:>8.2f}:1  {'✓' if ok else '✗ 不达标'}")
    return bad


def scan_css_source() -> int:
    """🔴 **源码扫描段：主题相关的颜色只允许出现在令牌块里。**

    为什么需要这一段（第 6 次「闸门绿得像没这回事」，最隐蔽的一次）
    ------------------------------------------------------------------
    B9 那一段亮色覆盖里写着 `.qx-hint-sparse { color: #8b96a3 }`，
    它**覆盖了 B0 令牌块的 `var(--qx-ink-hint)`（#5a6672，5.62:1）**，
    亮色下的稀疏提示实际渲染成 **2.88:1**。

    而对比度闸门当时报「✓ 全部达标」——
    因为它校验的是**令牌里声明的值**，不是**实际生效的规则**。

    > **校验的对象和渲染的对象不是同一个** ——
    > 令牌对了、被后面的规则覆盖掉了，闸门一无所知。

    判据：令牌块之外，任何给 `.qx-*` 元素设置**颜色字面量**的规则都是违规。
    主题色只允许从 `var(--qx-*)` 取。
    """
    css_path = ROOT / "qx-mobile" / "custom.css"
    src = css_path.read_text(encoding="utf-8")
    # 令牌块的行号范围（B0 注释开头 → .qx-layer 开始）
    start = src.find("B0 · 我们自己的颜色令牌")
    end = src.find(".qx-layer {", start)
    bad_lines: list[tuple[int, str]] = []
    in_token = False
    depth = 0
    for i, line in enumerate(src.splitlines(), 1):
        stripped = line.strip()
        # ⚠️ 跳过注释：注释里写 `.qx-hint-sparse { color: #8b96a3 }`
        #    只是**举例子说明曾经错在哪**，不是真规则。
        #    第一版扫描器没跳注释，报了一条不存在的违规 ——
        #    「闸门误报」和「闸门漏报」一样会让人不再相信它。
        if stripped.startswith(("/*", "*", "//", "#")):
            continue
        if start != -1 and start <= sum(len(l) + 1 for l in src.splitlines()[:i - 1]) < end:
            in_token = True
        if in_token and stripped == "}":
            in_token = False
            continue
        if in_token:
            continue
        if ".qx-" not in line:
            continue
        if not re.search(r"^\s*(\.[-\w]+|body|:root)[^{]*\{\s*$", line) and "{" not in line:
            continue
        if re.search(r"(^|\s)(color|background|background-color|border-color)\s*:\s*"
                     r"(#[0-9a-fA-F]{3,8}|rgb)", line):
            bad_lines.append((i, stripped))

    print(f"\n源码扫描段（{css_path.name}）" + "─" * 34)
    if not bad_lines:
        print("  ✓ 令牌块之外没有任何 .qx-* 颜色字面量")
        print("    （主题色全部走 var(--qx-*)）")
        return 0
    print(f"  ✗ 发现 {len(bad_lines)} 处：令牌块之外直接写了颜色字面量")
    for i, l in bad_lines:
        print(f"    第 {i} 行  {l}")
    print("\n  这类写法会**覆盖** B0 令牌块 —— 对比度闸门只看令牌，看不见它。")
    print("  请改成 var(--qx-*)。")
    return len(bad_lines)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--serial", default="", help="真机序列号（真机需开 WebView 调试）")
    ap.add_argument("--keep", action="store_true", help="结束后不切回亮色")
    args = ap.parse_args()
    serial = args.serial or None
    _guard_phone(serial)
    if not PROBE.exists():
        raise SystemExit(f"缺探针：{PROBE}")

    seen: dict[str, list] = {}
    dsh_bg: dict[str, tuple[float, float, float]] = {}
    can_switch = night(True, serial)
    if can_switch:
        time.sleep(3)
        print("── 深色 " + "─" * 50)
        collect(serial, "深色", seen, dsh_bg)
    if not args.keep and can_switch:
        night(False, serial)
        time.sleep(3)
    print("── 亮色 " + "─" * 50)
    collect(serial, "亮色", seen, dsh_bg)

    print()
    bad, missing = report(seen)
    bad += static_report(dsh_bg)
    bad += scan_css_source()
    if missing:
        print(f"\n⚠ 有 {len(missing)} 个目标整轮没在页面里量到：{'、'.join(missing)}")
        print("  它们已由静态段用**实测的 DSH 行背景**兜底核对（见上表）。")
    if bad == 0:
        print("\n✓ 全部达标（实测段 + 静态段）")
        return 0
    print(f"\n✗ {bad} 项不达标")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())