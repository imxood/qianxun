"""修 custom.js 的入口块：一次性重写，不做小步补丁。

背景（踩过的坑，值得单独一个脚本）
----------------------------------
第 2 版开头写成了：

    window.qxMobile.register((async ({document, window}) => { ... })())

语法**完全合法**，文件被网关**正常服务**，hash 也**正确更新**，
但界面一点反应都没有。

原因：`(async () => {...})()` 是**立即执行**的 IIFE，返回 **Promise**。
bootstrap.js:48 拿到 mount 的返回值当清理函数：

    const result = mount({ root: ensureRoot(), document, window })
    dispose = typeof result === 'function' ? result : null

Promise 不是 function -> dispose 永远是 null；而 mount 内部的逻辑
要等网络返回才跑 —— 于是按钮永远不出现。

这类失败的恶劣之处：**没有任何报错**。bootstrap 只在 catch 里打 console，
而它 catch 的是同步异常；一个「类型不对但没抛」的返回值一路静默通过。
唯一线索是「界面没动」，极易误判成 CSS 没生效。

正确写法：把入口改成**函数表达式**（不是声明，因为它在调用表达式里）：

    window.qxMobile.register(async function qxMount(ctx) { ... })

它在被调用时才执行，返回一个函数 —— 这正是 bootstrap 要的形状。
"""
from __future__ import annotations

import pathlib
import re

TARGET = pathlib.Path(
    r"E:\develop\dsh-workspace\qianxun\mobile-flutter\qx-mobile\custom.js"
)


def main() -> int:
    text = TARGET.read_text(encoding="utf-8")
    lines = text.splitlines()

    # ① 找入口起点：第一个真正的 window.qxMobile.register( （不在文件头注释里）
    start = None
    for index, line in enumerate(lines):
        if line.startswith("window.qxMobile.register("):
            start = index
            break
    if start is None:
        print("✗ 找不到入口 register( 行")
        return 1
    print(f"入口在第 {start + 1} 行")

    # ② 找结尾：最后一行应是 `})()`（闭合 IIFE），改成 `})`
    end = None
    for index in range(len(lines) - 1, start, -1):
        if lines[index].strip() == "})()":
            end = index
            break
    if end is None:
        print("✗ 找不到结尾 `})()`")
        return 1
    print(f"结尾在第 {end + 1} 行  `{lines[end].strip()}`")

    # ③ 报告当前的破损点
    body = lines[start + 1 : end]
    has_return_async = any("return async function" in l for l in body)
    has_iife_tail = any(l.strip() == "})()" for l in body)
    print(f"  体内有 return async function : {has_return_async}")
    print("  体内有多余的 IIFE 尾巴  :", has_iife_tail)
    if has_return_async and has_iife_tail:
        print("  → 这就是破损点：`return async function` 后面又跟了 `})()`，")
        print("    等于在函数体里嵌了一个 IIFE，语法直接崩。")

    # ④ 重建：入口 + 注释 + 干净的函数体
    note = [
        "  // ==================================================================",
        "  // 入口纪律：必须是**函数表达式**，不能是立即执行的 async 箭头。",
        "  //",
        "  // 踩过的坑（语法合法、服务正常、hash 正确，界面却完全不动）：",
        "  //     window.qxMobile.register((async (ctx) => { ... })())",
        "  // 那写法立即执行并返回 **Promise**。bootstrap.js:48 拿到 mount",
        "  // 的返回值当清理函数：",
        "  //     const result = mount({ root, document, window })",
        "  //     dispose = typeof result === 'function' ? result : null",
        "  // Promise 不是 function -> dispose 永远是 null，而 mount 内部的",
        "  // 逻辑要等网络返回才跑，于是按钮永远不出现。",
        "  //",
        "  // 恶劣之处：**没有任何报错**。bootstrap 只在 catch 里打 console，",
        "  // 而它 catch 的是同步异常；一个「类型不对但没抛」的返回值一路",
        "  // 静默通过。唯一线索是「界面没动」，极易误判成 CSS 没生效。",
        "  //",
        "  // 正确：register(async function qxMount(ctx) { ... })",
        "  // 返回函数本身，函数体在被调用时才跑。",
        "  // ==================================================================",
    ]

    # 去掉体内所有 `return async function ...` 与其后的 `})()` / `})`
    cleaned: list[str] = []
    skipping_tail = False
    for line in body:
        stripped = line.strip()
        if "return async function" in stripped:
            cleaned.append("  return async function qxMount({ document: doc, window: win }) {")
            skipping_tail = True
            continue
        if skipping_tail and stripped in {"})()", "})", "}"}:
            # 尾部被吃掉（函数真正的收尾在文件末尾统一加）
            continue
        skipping_tail = False
        cleaned.append(line)

    # 去掉尾部残留的孤立 `}`（之前那轮误删造成的）
    while cleaned and cleaned[-1].strip() in {"", "}"}:
        if cleaned[-1].strip() == "}":
            cleaned.pop()
        else:
            break

    new_lines = (
        ["window.qxMobile.register("]
        + note
        + cleaned
        + ["  }", "})"]
    )
    TARGET.write_text("\n".join(lines[:start] + new_lines) + "\n", encoding="utf-8")
    print(f"\n已重写入口块（第 {start + 1}–{end + 1} 行）")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
