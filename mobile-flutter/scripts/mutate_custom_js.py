"""造一个「故意改坏的 custom.js」，验证行为闸**抓得住**。

为什么要专门做这个
----------------
本轮踩到两次「代码跑了但一条都没命中」：
  1. 弹窗抑制在 `div,section,aside,dialog` 里找最小容器，
     而文案在 `<p>` 里 → 命中 0 个，弹窗照旧
  2. 侧栏行按标题匹配 → 4 条同名会话张冠李戴

两次都是「看起来在跑」。闸门如果从没红过，就不能证明它在防什么。
所以这里把修复前的写法塞回去，看闸门报不报。

用法：python .tmp/mutate_gate.py
"""
from __future__ import annotations

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GOOD = ROOT / "qx-mobile" / "custom.js"
MUT = ROOT / ".tmp" / "_mutant_custom.js"
GATE = ROOT / "scripts" / "check_custom_js_runs.py"

# 变异 1：把 data-row-key 精确匹配改回第 3 版的标题匹配
MUTATE_ID = """  function findByRow(row) {
    const title = textOf(row).split('\\n')[0].trim()
    if (!title) return null
    let best = null
    state.sessions.forEach((s) => {
      if (s.title && s.title.slice(0, 12) === title.slice(0, 12)) {
        if (!best || s.updatedAt > best.updatedAt) best = s;
      }
    })
    return best || null
  }"""

# 变异 2：把整段 suppressPreviewNotice 换回第 3 版的「叶子查找」写法。
# ⚠️ 必须换**整段函数**，不能只换 querySelectorAll 那一行 ——
#    只换选择器的话函数体还是第 4 版的，照样能命中，闸门自然红不了。
#    （第一版变异就是这么写的，结果白跑一趟，还差点误判「闸门没用」。）
MUTATE_NOTICE = """  function suppressPreviewNotice() {
    const marks = ['Preview Notice', 'still in preview', '预览版']
    let found = 0
    const all = doc.querySelectorAll('div, section, aside, dialog')
    for (let i = 0; i < all.length; i += 1) {
      const candidate = all[i]
      const t = (candidate.textContent || '').trim()
      if (!t || t.length > 900) continue
      let matched = false
      for (let m = 0; m < marks.length; m += 1) {
        if (t.indexOf(marks[m]) !== -1) { matched = true; break }
      }
      if (!matched) continue
      const hasChild = Array.prototype.some.call(
        candidate.children || [],
        (c) => marks.some((mark) => (c.textContent || '').indexOf(mark) !== -1),
      )
      if (hasChild) continue
      candidate.style.setProperty('display', 'none', 'important')
      found += 1
    }
    return found
  }"""

# 当前（第 4 版）整段函数，变异时整段替换
OLD_NOTICE = """  function suppressPreviewNotice() {
    const marks = ['Preview Notice', 'still in preview', '预览版']
    let found = 0
    const dialogs = doc.querySelectorAll('[role="dialog"], [aria-modal="true"]')"""


# 变异 3：把「统计区」的判据退回「数按钮个数 + 取最内层合格祖先」。
# 对应真实缺陷：只包住两颗药丸的 `bOPqWQ_root` 被选中，
# 而 79% 圆环是它的**兄弟** —— 结果药丸藏了、圆环留在屏幕底部。
#
# ⚠️ 变异必须复现**真实发生过的那次**：早先一版写成「一路覆盖成最外层」，
#    而那恰好与现在的实现等价，闸门自然红不了 ——
#    **变异体自己也要先确认它真的改变了行为**。
MUTATE_STATROW = """  function findStatRow() {
    const bar = doc.querySelector('[data-slot="conversation.composer.bar"]')
    if (!bar) return null
    const btns = findStatButtons()
    if (btns.length < 2) return null
    for (const b of btns) {
      let n = b.parentElement
      for (let i = 0; i < 3 && n && n !== bar; i += 1, n = n.parentElement) {
        let k = 0
        for (const c of n.children) {
          if (btns.some((x) => n.contains(x) && c.contains(x))) k += 1
        }
        if (k >= 2) return n
      }
    }
    return null
  }"""

# 变异 4：百分比改成宽泛的 /(\d+)%/（去掉 ^…$ 的整段锚定）。
# 对应真实缺陷：会先撞上「89.2M tok · Cache hit 98%」，
# 把缓存命中率当成上下文占用显示 —— 数字大得吓人，含义也全错。
MUTATE_PCT = "      const m = /(\\d{1,3})\\s*%/.exec(t)"

# 变异 5：把状态按钮 append 到行尾（而不是插到 trailing 之前）。
# 对应真实缺陷：读数落到「发送」键右边，把次要信息放在主操作旁边。
MUTATE_APPEND = """    if (anchor) row.insertBefore(btn, anchor)
    else if (rightGroup && rightGroup.parentElement === row) row.insertBefore(btn, rightGroup)
    else row.appendChild(btn)"""
MUTATE_APPEND_BAD = "    row.appendChild(btn)"

# 变异 6：让 findStatButtons 认自己的按钮。
# 对应真实缺陷：药丸文字是「79%」，同样命中统计判据 →
# 标记一路走到 composer 根，B12 把**整个输入框**藏起来。
MUTATE_SELF = """      if (b.closest('.qx-status-btn')) continue
"""


def replace_block(src: str, start: str, new_block: str) -> str | None:
    """从 start 处开始，按花括号配平截出整个函数，换成 new_block。"""
    i = src.find(start)
    if i == -1:
        return None
    j = src.find("{", i)
    depth = 0
    for k in range(j, len(src)):
        if src[k] == "{":
            depth += 1
        elif src[k] == "}":
            depth -= 1
            if depth == 0:
                return src[:i] + new_block + src[k + 1:]
    return None


def run(src: str, old: str, new: str, label: str) -> int:
    text = src
    if old not in text:
        print(f"[{label}] 没能定位到待变异代码 —— 闸门脚本本身该更新了")
        return 2
    text = text.replace(old, new, 1)
    MUT.write_text(text, encoding="utf-8")
    proc = subprocess.run(
        [sys.executable, str(GATE), str(MUT)],
        capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=120,
    )
    out = proc.stdout.strip().splitlines()
    verdict = "🔴 闸门抓到了" if proc.returncode != 0 else "🟢 闸门漏了 —— 这道闸没用"
    print(f"[{label}] {verdict}")
    for line in out:
        if line.startswith(("FAIL", "PASS")):
            print("   " + line)
    print()
    return 0 if proc.returncode != 0 else 1


def run_text(text: str, label: str) -> int:
    MUT.write_text(text, encoding="utf-8")
    proc = subprocess.run(
        [sys.executable, str(GATE), str(MUT)],
        capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=120,
    )
    out = proc.stdout.strip().splitlines()
    verdict = "🔴 闸门抓到了" if proc.returncode != 0 else "🟢 闸门漏了 —— 这道闸没用"
    print(f"[{label}] {verdict}")
    for line in out:
        if line.startswith(("FAIL", "PASS")):
            print("   " + line)
    print()
    return 0 if proc.returncode != 0 else 1


def main() -> int:
    src = GOOD.read_text(encoding="utf-8")
    bad = 0
    bad |= run(
        src,
        "  function findByRow(row) {\n"
        "    const id = sessionIdOfRow(row)\n"
        "    if (!id) return null\n"
        "    return state.sessions.get(id) || null\n  }",
        MUTATE_ID,
        "变异1 · 退回标题匹配",
    )
    mutated = replace_block(src, OLD_NOTICE, MUTATE_NOTICE)
    if mutated is None:
        print("[变异2] 没能定位到 suppressPreviewNotice")
        return 2
    bad |= run_text(mutated, "变异2 · 退回叶子查找")

    # ── 第 6 版新增的四个变异（状态条收进按钮那一轮）─────────────────
    mutated = replace_block(src, "  function findStatRow() {", MUTATE_STATROW)
    if mutated is None:
        print("[变异3] 没能定位到 findStatRow")
        bad |= 1
    else:
        bad |= run_text(mutated, "变异3 · 统计区退回「数按钮个数」")

    bad |= run(
        src,
        "      const m = /^(\\d{1,3})\\s*%$/.exec(t)",
        MUTATE_PCT,
        "变异4 · 百分比去掉整段锚定（会抓到缓存命中率）",
    )

    bad |= run(src, MUTATE_APPEND, MUTATE_APPEND_BAD,
               "变异5 · 状态按钮 append 到行尾（跑到发送键右边）")

    bad |= run(src, MUTATE_SELF, "",
               "变异6 · findStatButtons 认自己的按钮（会藏掉整个输入框）")

    # ── 第 9 版：两个变异，分别对应本轮查明的两处机制 ─────────────────
    # 变异7：退回 `>= 32`。这是**真机上真实发生的卡死**：
    #   按钮被设成 32 后，下一轮量到 32 就不再匹配 → 永远升不上 44，
    #   而每次扫描都「跑过了」。只看真机截图很容易误以为是自己没刷。
    bad |= run(src, "      if (r.width >= 44) continue",
               "      if (r.width >= 32) continue",
               "变异7 · 门槛退回 32（按钮卡死在 32，永远升不到 44）")

    # 变异8：去掉 important。**值完全正确，但在真机上无效** ——
    #   行内 !important 才能压过 DSH 那条优先级更高的声明。
    #   这类变异最危险：本地测试全绿、真机纹丝不动。
    bad |= run(
        src,
        "      el.style.setProperty('min-width', '44px', 'important')\n"
        "      el.style.setProperty('min-height', '44px', 'important')",
        "      el.style.setProperty('min-width', '44px')\n"
        "      el.style.setProperty('min-height', '44px')",
        "变异8 · 行内 min-* 漏掉 !important（值对，真机无效）",
    )

    # ── 第 10 版：Trajectory 页的两处 ──────────────────────────────────
    # 变异9：把「工具条里的矮按钮」那条判据删掉。
    #   少了它，Trajectory 顶栏的 "Collapse turns"（55×20）
    #   和 "Collapse calls"（51×20）永远补不到 44 ——
    #   它们宽度不够 45%、父容器高 44 与自身 20 相差 24>3，两条老判据都匹配不上。
    bad |= run(
        src,
        "        else if (ph && ph >= r.height && ph <= 60) {\n"
        "          const pcs = win.getComputedStyle(p)\n"
        "          if (pcs.display === 'flex' || pcs.display === 'inline-flex') wideEnough = true\n"
        "        }\n",
        "",
        "变异9 · 删掉「工具条里的矮按钮」判据（Trajectory 顶栏 20px 高的按钮补不到 44）",
    )

    # 变异10：请求标记只补尺寸、不补对称负边距。
    #   值完全正确、热区确实变成 44 —— 但**布局盒子也从 16 变成 44**：
    #   行高 30 → 58，14 行就是 +392px，滚动条明显变短，圆点也会挤成一列。
    #   这是「只做了一半」的典型：截图看起来变大了，其实是布局塌了。
    bad |= run(
        src,
        "      el.style.setProperty('margin-left', (-dx) + 'px', 'important')\n"
        "      el.style.setProperty('margin-right', (-dx) + 'px', 'important')\n"
        "      el.style.setProperty('margin-top', (-dy) + 'px', 'important')\n"
        "      el.style.setProperty('margin-bottom', (-dy) + 'px', 'important')",
        "      el.style.setProperty('margin-left', (-dx) + 'px', 'important')",
        "变异10 · 请求标记漏掉对称负边距（热区对了但布局盒子被撑成 44）",
    )

    # 变异11：把「只拉大透明无子元素的小方块」放宽成「所有 ≤20 的按钮」。
    #   后果是误伤 12×12 的颜色块和带图标的真按钮 —— 布局会被撑乱。
    #   这条变异盯的是**判据的另一半**：闸门必须同时证明「能修」和「不误伤」。
    bad |= run(
        src,
        "      if (el.children.length) continue",
        "      // 变异：去掉「无子元素」与「透明背景」两道限制",
        "变异11 · 判据放宽到所有小方块（误伤色块和真按钮）",
    )

    # 变异12：去掉「mount 先清掉上一次遗留的层」。
    #   后果是热重载若干次后页面上堆着 2、3、4…个 .qx-layer。
    #   它们 pointer-events:none、没有子元素，谁也挡不住 ——
    #   **肉眼看不出、截图拍不出、功能测试全绿**，
    #   只有一个数会变：数量。而原来的断言只问「在不在」。
    bad |= run(
        src,
        "  for (const stale of doc.querySelectorAll('.qx-layer')) stale.remove()\n",
        "",
        "变异12 · mount 不清遗留层（热重载后堆出多个 qx-layer，肉眼无感）",
    )

    MUT.unlink(missing_ok=True)
    return 0 if bad == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
