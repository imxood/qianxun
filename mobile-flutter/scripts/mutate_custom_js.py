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
    MUT.unlink(missing_ok=True)
    return 0 if bad == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
