#!/usr/bin/env python3
"""校验色板的 WCAG 对比度。

设计文档 05 §9.2 的验收项之一：**状态色作为正文文字压在 canvas 上
对比度 ≥ 4.5:1，作为色块对 canvas ≥ 3:1**。这里直接算，不靠眼睛。

改色板后跑一遍::

    python scripts\\check_contrast.py
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

THEME = Path(__file__).resolve().parent.parent / "lib" / "app" / "theme" / "qx_semantic.dart"
LIB = Path(__file__).resolve().parent.parent / "lib"

# `onAccent` 的用法扫描窗口：多少行之内必须能看到 accent 底。
# 取 8 是因为典型写法是
#     color: enabled ? q.accent : q.surfaceRaised,   // 底
#     ...
#     color: enabled ? q.onAccent : q.textTertiary,  // 字
ON_ACCENT_WINDOW = 8


def check_on_accent_usage() -> list[str]:
    """扫源码：每一处 `onAccent` 的附近必须有 accent 底。

    为什么不能做成**色板**断言：`onAccent` 是配对色，它在 canvas / surface
    上必然是 1.0x:1 —— 那正是它成立的前提。断言「它必须在这些面上 ≥4.5」
    会每次都红，闸门立刻变成摆设（要么被删，要么被人 `--fix` 成另一个
    颜色，而真正的问题「配对色被当独立色用」一点没解决）。

    真正的不变式是**用法**：`onAccent` 只能出现在 accent 底上。
    日志级别 INFO 曾写 `QxLevel.info => q.onAccent`，级别竖条与选中的
    筛选 chip 文字双双隐形，就是绕过了这个不变式。
    """
    problems: list[str] = []
    # 底色引用：出现 accent 且**不是** onAccent 本身。
    accent_bg = re.compile(r"(?<!on)accent", re.IGNORECASE)
    for path in sorted(LIB.rglob("*.dart")):
        if path.name == "qx_semantic.dart":
            continue  # 令牌定义本身不算「用法」
        lines = path.read_text(encoding="utf-8").splitlines()
        for i, line in enumerate(lines):
            stripped = line.lstrip()
            if "onAccent" not in line or stripped.startswith(("//", "*")):
                continue
            # 作用域 = 从**最近的缩进 0 行**（顶层声明：class / 函数 / 变量）
            # 到本行。底色与前景色常隔着整个 widget 结构（发送键的 accent 底
            # 在 25 行之上），固定行窗口会误报，所以按声明边界切。
            #
            # ⚠️ 只认缩进 0：曾经改成「找最近的 class/@override/build」，
            # 结果顶层函数 `_levelColor` 里的误用会一路回溯到**上一个类**的
            # @override，把不相干的 accent 算了进来 → 漏报。
            lo = i
            while lo > 0 and i - lo < 80:
                if lines[lo] and not lines[lo][0].isspace():
                    break
                lo -= 1
            window = "\n".join(lines[lo : i + 1])
            if accent_bg.search(window):
                continue
            rel = path.relative_to(LIB.parent)
            problems.append(
                f"{rel}:{i + 1} 用了 onAccent，但所在声明内没有 accent 底："
                f"{stripped[:70]}"
            )
    return problems

# WCAG 2.x 相对亮度
def luminance(hex_color: str) -> float:
    h = hex_color.lstrip("#")
    r, g, b = (int(h[i : i + 2], 16) / 255 for i in (0, 2, 4))

    def lin(c: float) -> float:
        return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4

    return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)


def ratio(a: str, b: str) -> float:
    la, lb = luminance(a), luminance(b)
    hi, lo = max(la, lb), min(la, lb)
    return (hi + 0.05) / (lo + 0.05)


def extract(dart: str, name: str) -> dict[str, tuple[str, float]]:
    """从 `QxSemantic` 的 `dark` / `light` 常量里抠出所有 Color(0x…) 的字段。

    ⚠️ Dart 的 `Color(0xFF0B0E13)` 是 **ARGB**，前两位是 alpha。
    取颜色必须丢掉 alpha 字节（`argb[2:]`），只保留后 6 位 RGB。
    曾经错取成 `val[2:]` 导致把 alpha 当成 R，算出来的对比度全是错的。

    刻意**允许非 FF 的 alpha**：`hairline` 就是半透明描边。
    返回 (rgb, alpha)，由调用方在需要时合成到承载面上。
    """
    out: dict[str, tuple[str, float]] = {}
    m = re.search(rf"static const {name} = QxSemantic\((.*?)\n  \);", dart, re.S)
    if not m:
        return out
    for field, val in re.findall(r"(\w+):\s*Color\((0x[0-9A-Fa-f]{8})\)", m.group(1)):
        argb = val[2:]  # 去掉 0x
        alpha = int(argb[:2], 16) / 255
        out[field] = ("#" + argb[2:], alpha)  # 丢掉 alpha，只留 RGB
    return out


def flatten(p: dict[str, tuple[str, float]]) -> dict[str, str]:
    """把带 alpha 的颜色合成到 canvas 上，得到可直接参与对比度计算的不透明值。"""
    canvas = p["canvas"][0]
    out: dict[str, str] = {}
    for k, (rgb, a) in p.items():
        if a >= 0.999:
            out[k] = rgb
        elif a <= 0.001:
            out[k] = canvas
        else:
            cr, cg, cb = (int(canvas[i : i + 2], 16) for i in (1, 3, 5))
            r, g, b = (int(rgb[i : i + 2], 16) for i in (1, 3, 5))
            mix = lambda x, y: round(x * a + y * (1 - a))
            out[k] = "#%02X%02X%02X" % (mix(r, cr), mix(g, cg), mix(b, cb))
    return out


TEXT_MIN = 4.5   # 正文 WCAG AA
UI_MIN = 3.0     # 非文字（色块、描边）WCAG AA


def main() -> int:
    src = THEME.read_text(encoding="utf-8")
    raw = {"dark": extract(src, "dark"), "light": extract(src, "light")}
    # 解析失败必须**大声失败**。旧版只检查 `dark`，`light` 匹配不到时会
    # 抛一个裸的 KeyError，看起来像崩溃而不是「色板没解析到」。
    for name in ("dark", "light"):
        if not raw[name]:
            print(
                f"✗ 没从 {THEME} 解析出 {name} 色板。"
                f"检查 `static const {name} = QxSemantic(` 的格式是否变了。",
                file=sys.stderr,
            )
            return 2
    # 半透明色（hairline）先合成到 canvas 上再比
    palettes = {k: flatten(v) for k, v in raw.items()}
    required = {
        "running", "waiting", "done", "detached", "failed", "accent",
        "onAccent", "canvas", "surface", "surfaceRaised", "surfaceSunken",
        "textPrimary", "textSecondary", "textTertiary",
        "railIdle", "onSurfaceVariant",
    }
    for name, p in palettes.items():
        missing = required - p.keys()
        if missing:
            print(
                f"✗ {name} 色板缺字段：{sorted(missing)}。"
                f"色板与脚本不同步，补上或更新 required 集合。",
                file=sys.stderr,
            )
            return 2

    failures: list[str] = []
    for mode, p in palettes.items():
        canvas = p["canvas"]
        surface = p["surface"]
        print(f"\n=== {mode} ===")
        print(f"canvas  {canvas}   surface {surface}   surfaceRaised {p['surfaceRaised']}")

        # 1) 状态色作为**正文文字**压在 canvas 上：≥ 4.5:1
        print("\n-- 状态色作正文文字 vs canvas（需 ≥ 4.5）--")
        for role in ("running", "waiting", "done", "detached", "failed", "accent"):
            r = ratio(p[role], canvas)
            ok = r >= TEXT_MIN
            print(f"  {role:<9} {p[role]}  {r:5.2f}:1  {'OK' if ok else 'FAIL'}")
            if not ok:
                failures.append(f"{mode}.{role} 作正文文字 {r:.2f}:1 < 4.5")

        # 2) 状态色作为**色块**对 canvas：≥ 3:1
        print("\n-- 状态色作色块 vs canvas（需 ≥ 3.0）--")
        for role in ("running", "waiting", "done", "detached", "failed"):
            r = ratio(p[role], canvas)
            ok = r >= UI_MIN
            print(f"  {role:<9} {r:5.2f}:1  {'OK' if ok else 'FAIL'}")
            if not ok:
                failures.append(f"{mode}.{role} 作色块 {r:.2f}:1 < 3.0")

        # 3) 文字三级对**各自真正承载它的面**。
        #    ⚠️ 曾经只查 textTertiary on surface，结果漏掉了它作为
        #    `hintStyle` 压在 `surfaceSunken`（输入框底）上的场景。
        print("\n-- 文字三级 on surface（需 ≥ 4.5）--")
        for role in ("textPrimary", "textSecondary", "textTertiary"):
            r = ratio(p[role], p["surface"])
            ok = r >= TEXT_MIN
            print(f"  {role:<13} on surface     {r:5.2f}:1  {'OK' if ok else 'FAIL'}")
            if not ok:
                failures.append(f"{mode}.{role} on surface {r:.2f}:1 < 4.5")

        print("\n-- 文字三级 on surfaceSunken / canvas（需 ≥ 4.5）--")
        for bg in ("surfaceSunken", "canvas"):
            for role in ("textSecondary", "textTertiary"):
                r = ratio(p[role], p[bg])
                ok = r >= TEXT_MIN
                print(f"  {role:<13} on {bg:<14} {r:5.2f}:1  {'OK' if ok else 'FAIL'}")
                if not ok:
                    failures.append(f"{mode}.{role} on {bg} {r:.2f}:1 < 4.5")

        # 4) 徽标：状态词是**10.5px 粗体**，小字需要的对比度并不比正文低。
        #    曾经 StateBadge 铺了一层 14% 的同色底，把状态色压到 4.0 左右
        #    ——五个状态在亮色下全部不达标，而旧脚本只看「状态色 vs canvas」，
        #    结构上看不见。
        print("\n-- 徽标底（已改为无底色，只留描边）--")
        print("  描边色 = 状态色 @45%，仅作装饰，不承载文字")
        print("  文字色 = 状态色原值，已在上面「vs canvas」一项验过")

        # 5) accent 上的前景色。暗色的 accent 是亮青，前景必须是深色；
        #    曾经硬写 Colors.white 只有 2.56:1。
        print("\n-- accent 上的前景色（需 ≥ 4.5）--")
        r = ratio(p["onAccent"], p["accent"])
        ok = r >= TEXT_MIN
        print(f"  onAccent on accent   {r:5.2f}:1  {'OK' if ok else 'FAIL'}")
        if not ok:
            failures.append(f"{mode}.onAccent on accent {r:.2f}:1 < 4.5")

        # 5b) ⚠️ `onAccent` **只能**用在 accent 底上。
        #
        # 它是**配对色**，低对比正是它的定义（所以下面这几行必然是低值，
        # 只作参考打印，**不计入 failures**）。真正的风险在用法：
        # 日志级别 INFO 曾取 `q.onAccent`，于是级别竖条看不见、
        # 选中的筛选 chip 文字也看不见（1.1:1 上下）。
        #
        # ⚠️ 因此本项**不能**做成色板断言 —— 那样它每次都失败，闸门就废了。
        # 断言在 `check_on_accent_usage()`：扫源码，要求每一处 `onAccent`
        # 附近确实有 accent 底。
        print("\n-- onAccent 作独立色用的风险值（仅参考，配对色本就低对比）--")
        for bg in ("canvas", "surface", "surfaceRaised", "surfaceSunken"):
            r = ratio(p["onAccent"], p[bg])
            print(f"  onAccent on {bg:<14} {r:5.2f}:1  ← 单独用就是这个结果")

        # 5c) `onSurfaceVariant` 是 30+ 处 Material 次要文字的真实取值，
        #     必须和三级文字一样过线 —— 它曾被 seed 推导成带青绿的值。
        print("\n-- onSurfaceVariant（Material 次要文字，需 ≥ 4.5）--")
        for bg in ("surface", "surface", "canvas", "surfaceSunken", "surfaceRaised"):
            r = ratio(p["onSurfaceVariant"], p[bg])
            ok = r >= TEXT_MIN
            print(f"  onSurfaceVariant on {bg:<14} {r:5.2f}:1  {'OK' if ok else 'FAIL'}")
            if not ok:
                failures.append(f"{mode}.onSurfaceVariant on {bg} {r:.2f}:1 < 4.5")

        # 5d) 轨专用色与代码底。
        #
        # `railIdle` 的意图是「已完成」的 1dp 轨**退到几乎看不见**，
        # 所以它**不要求** 3:1（那正是它的设计目标）。但仍然要求它
        # **看得见**：低于 1.3:1 就不是「退后」而是「消失」了。
        # `codeBackground` 承载工具输出文字，必须真的能读。
        print("\n-- railIdle（1dp 轨，退后但不消失：需 ≥ 1.3）--")
        r = ratio(p["railIdle"], p["surface"])
        ok = r >= 1.3
        print(
            f"  railIdle on surface {r:5.2f}:1  "
            f"{'OK' if ok else 'FAIL ← 太低就不是退后而是消失'}"
        )
        if not ok:
            failures.append(f"{mode}.railIdle on surface {r:.2f}:1 < 1.3（会完全看不见）")
        # 代码块底色就是 surfaceSunken（`codeBackground` 令牌已删：两套色
        # 都与它同值、零调用点），所以文字在代码底上的可读性由上面
        # 「文字三级 on surfaceSunken」那一项保证，不重复验。

        # 6) 三级文字必须**单调**，否则层级是反的。
        print("\n-- 文字三级单调性（primary 最强 → tertiary 最弱）--")
        for bg, invert in (("surface", True), ("canvas", True), ("surfaceSunken", True)):
            chain = ["textPrimary", "textSecondary", "textTertiary"]
            vals = [ratio(p[c], p[bg]) for c in chain]
            good = all(vals[i] > vals[i + 1] for i in range(len(vals) - 1))
            print(
                f"  on {bg:<14} "
                + " > ".join(f"{c}:{v:.2f}" for c, v in zip(chain, vals))
                + f"  {'OK' if good else 'FAIL'}"
            )
            if not good:
                failures.append(f"{mode} 文字三级在 {bg} 上不是单调递减")

        # 4) 表面层级必须**单调有序**。
        #
        # ⚠️ 这里刻意**不**要求 1.6x 那种大跳变。本 App 的层级靠**发丝
        # 描边**承载（OLED 上阴影几乎不可见），表面之间本来就该挨得很近；
        # 要求大幅亮度差等于要求用阴影做层级，那是反过来的做法。
        # 真正要保证的是**方向正确**：raised 比 surface 亮、sunken 比
        # surface 暗，顺序不能反。
        print("\n-- 表面层级顺序（raised>surface>sunken，方向不能反）--")
        la = luminance(p["surfaceRaised"])
        lb = luminance(p["surface"])
        lc = luminance(p["surfaceSunken"])
        for label, cond, ok in (
            ("surfaceRaised > surface", la > lb, la > lb),
            ("surfaceSunken < surface", lc < lb, lc < lb),
        ):
            print(f"  {label:<26} {'OK' if ok else 'FAIL'}")
            if not ok:
                failures.append(f"{mode} 表面层级顺序反了: {label}")

    # 7) 配对色不得被当独立色用（源码扫描）。
    print("\n-- onAccent 用法扫描（配对色只能压在 accent 底上）--")
    usage_problems = check_on_accent_usage()
    if usage_problems:
        print(f"  ✗ {len(usage_problems)} 处 onAccent 附近没有 accent 底：")
        for p in usage_problems:
            print(f"      {p}")
        failures.extend(usage_problems)
    else:
        print("  OK  每处 onAccent 都在 accent 底上")

    print("\n" + "=" * 60)
    if failures and "--fix" in sys.argv:
        print("✗ 不达标项，以及自动求解出的替代色：\n")
        for f in failures:
            print(f"  {f}")
            print(solve(f, palettes))
        print("\n把上面这些值填回 qx_semantic.dart 再跑一次。")
        return 1
    if failures:
        print(f"✗ {len(failures)} 项不达标：")
        for f in failures:
            print(f"   {f}")
        return 1
    print("✓ 全部达标")
    return 0


def solve(failure: str, palettes: dict[str, dict[str, str]]) -> str:
    """沿明度轴二分搜索，直到满足该项的对比度阈值。

    做法：朝「对比更高的方向」逐轮调整 HSL 的 L，保持色相与饱和度不变。
    这样修出来的颜色仍然是**同一个色相**（绿还是绿、琥珀还是琥珀），
    不会为了凑对比度把语义色变成别的颜色。
    """
    import colorsys

    mode = failure.split(".")[0]
    p = palettes[mode]
    if "作正文文字" in failure:
        role, target = failure.split(".")[1].split(" 作正文文字")[0], TEXT_MIN
        bg = p["canvas"]
    elif "作色块" in failure:
        role, target = failure.split(".")[1].split(" 作色块")[0], UI_MIN
        bg = p["canvas"]
    else:
        return "  （不是色值问题，人工处理）"

    h, l, s = _hex_to_hsl(p[role])
    best = p[role]
    for i in range(1, 101):
        cand = l + (i / 100) * (1 - l) if luminance(bg) < 0.5 else l - (i / 100) * l
        if ratio(_hsl_to_hex(h, max(0.0, min(1.0, cand)), s), bg) >= target:
            best = _hsl_to_hex(h, max(0.0, min(1.0, cand)), s)
            break
    got = ratio(best, bg)
    return f"    {mode}.{role}: {p[role]} → {best}  ({got:.2f}:1 ≥ {target})"


def _hex_to_hsl(hx: str):
    import colorsys

    r, g, b = (int(hx[i : i + 2], 16) / 255 for i in (1, 3, 5))
    return colorsys.rgb_to_hls(r, g, b)


def _hsl_to_hex(h: float, l: float, s: float) -> str:
    import colorsys

    r, g, b = colorsys.hls_to_rgb(h, l, s)
    return "#%02X%02X%02X" % (round(r * 255), round(g * 255), round(b * 255))


if __name__ == "__main__":
    sys.exit(main())
