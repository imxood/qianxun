"""抓窄屏布局与可见锚点，落盘 docs/probe/narrow-layout.json。

为什么要有它
------------
「DSH 侧栏在窄屏下把主区挤成 132px」「DSH 暴露的是 data-slot 而不是 class」
这两条，第 4 版是**对照真机 DOM 量出来的**，不是看文档看出来的。
不落盘的话，下一轮就只能靠注释转述 —— 而本轮已经吃过一次亏：
第 2 版那 12 组「窄屏适配」用的类名命中 0 个元素，
却在提交信息里被写成「已部署」。

所以规则是：**CSS 规则与选择器纪律，结论必须有命中数**。

它记什么
--------
1. `data-slot` 全量枚举（哪些锚点真的存在）
2. 全部 `data-*` 属性（class 之外的可见锚点清单）
3. `[data-slot=root] > *` 的 data-* 状态属性 + grid 列（展开/折叠两态）
4. 侧栏 / 主区 / 输入框的实际盒模型

用法：python scripts/probe_narrow_layout.py
"""
from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CDP = ROOT / "scripts" / "cdp_webview.py"
OUT = ROOT / "docs" / "probe" / "narrow-layout.json"

EXPR = r"""(()=>{
  const slots = {};
  for (const e of document.querySelectorAll('[data-slot]')) {
    const v = e.getAttribute('data-slot');
    slots[v] = (slots[v] || 0) + 1;
  }
  const dataAttrs = new Set();
  for (const e of document.querySelectorAll('*')) {
    for (const a of e.attributes) {
      if (a.name.startsWith('data-') && a.name !== 'data-slot') dataAttrs.add(a.name);
    }
  }
  const frame = document.querySelector('[data-slot="root"] > *');
  const side = document.querySelector('[data-slot="sidebar"] > *');
  const box = (n) => {
    if (!n) return null;
    const cs = getComputedStyle(n);
    const r = n.getBoundingClientRect();
    return {
      display: cs.display, position: cs.position, zIndex: cs.zIndex,
      w: Math.round(r.width), h: Math.round(r.height), x: Math.round(r.x),
    };
  };
  const leafWithText = (re) => [...document.querySelectorAll('*')]
    .filter(e => e.children.length === 0 && re.test(e.textContent || '')).length;
  return JSON.stringify({
    viewport: { w: window.innerWidth, h: window.innerHeight },
    slots,
    dataAttrs: Array.from(dataAttrs).sort(),
    frame: {
      hasDataSidebarCollapsed: frame ? frame.hasAttribute('data-sidebar-collapsed') : null,
      inlineStyle: frame ? frame.getAttribute('style') : null,
      gridColumns: frame ? getComputedStyle(frame).gridTemplateColumns : null,
      box: box(frame),
    },
    sidebar: { box: box(side) },
    treeItems: document.querySelectorAll('[role=treeitem]').length,
    // 「桌面侧栏到底有没有『未分组』组」—— 全页叶子节点扫描
    ungroupedLeaves: leafWithText(/未分组|Ungrouped/),
    // 「工具行锚点是否存在」—— 没有工具输出的会话里它不会出现
    disclosureRows: document.querySelectorAll('[data-disclosure-row]').length,
  });
})()"""


def cdp_eval(expr: str) -> dict:
    tmp = ROOT / ".tmp" / "_probe_narrow.js"
    tmp.write_text(expr, encoding="utf-8")
    proc = subprocess.run(
        [sys.executable, str(CDP), "eval", "--file", str(tmp)],
        capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=180,
    )
    tail = [ln for ln in proc.stdout.strip().splitlines() if ln.strip().startswith("{")]
    if not tail:
        raise SystemExit(f"没拿到结果：\n{proc.stdout}\n{proc.stderr}")
    return json.loads(tail[-1])


def main() -> int:
    data = cdp_eval(EXPR)
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(data, ensure_ascii=False, indent=2), encoding="utf-8")

    print(f"视口 {data['viewport']['w']}×{data['viewport']['h']}")
    print(f"data-slot {len(data['slots'])} 种，data-* 锚点 {len(data['dataAttrs'])} 种")
    print(f"  frame 折叠态={data['frame']['hasDataSidebarCollapsed']} "
          f"列={data['frame']['gridColumns']}")
    print(f"  侧栏 {data['sidebar']['box']['w']}px "
          f"position={data['sidebar']['box']['position']}")
    print(f"  treeitem {data['treeItems']} · "
          f"未分组叶子 {data['ungroupedLeaves']} · "
          f"工具行 {data['disclosureRows']}")
    print(f"→ {OUT.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
