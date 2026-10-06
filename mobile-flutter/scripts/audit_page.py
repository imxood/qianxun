"""页面布局自动审查 —— 每一轮迭代跑一次，把「肉眼容易漏」的问题量化出来。

为什么不靠看截图
----------------
看图只能发现最刺眼的那几个问题，而「窄屏不适配」绝大多数是：
  · 横向溢出（某一列没 min-width:0，把整页撑宽）
  · 触控目标 < 44dp（看得见但点不准）
  · 文案被省略号砍掉（用户不知道后面还有内容）
  · 元素重叠 / 跑出视口
  · 正文字号 < 12px（真机上根本看不清）
这些都能用 DOM + getBoundingClientRect 量出来，比人眼可靠得多。

⚠️ 模拟器已被调成 **与真机一致**（1256×2760 @ 560dpi = 359×789dp, dpr 3.5），
   所以这里量到的结论可以直接搬到真机。

用法：
  python scripts/audit_page.py            # 审查当前页面
  python scripts/audit_page.py --json     # 机器可读
"""
from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CDP = ROOT / "scripts" / "cdp_webview.py"

JS = r"""
(() => {
  const VW = window.innerWidth, VH = window.innerHeight;
  const out = {
    viewport: { w: VW, h: VH, dpr: devicePixelRatio },
    docScrollW: document.documentElement.scrollWidth,
    bodyScrollW: document.body.scrollWidth,
    overflowX: document.documentElement.scrollWidth > VW + 1,
    problems: [],
    info: {},
  };
  const add = (kind, sev, msg, el) => out.problems.push({
    kind, sev, msg,
    box: el ? (() => { const r = el.getBoundingClientRect();
      return [Math.round(r.x), Math.round(r.y), Math.round(r.width), Math.round(r.height)]; })() : null,
    tag: el ? el.tagName + (el.className ? '.' + String(el.className).slice(0, 40) : '') : null,
  });

  // 只看「用户看得见的」元素：排除 display:contents / 0 尺寸 / 隐藏
  const vis = [];
  for (const el of document.querySelectorAll('body *')) {
    if (el.closest('svg')) continue;
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden' || +cs.opacity === 0) continue;
    const r = el.getBoundingClientRect();
    if (r.width < 1 || r.height < 1) continue;
    if (el.children.length === 0 && !(el.innerText || '').trim()) continue;
    vis.push({ el, cs, r });
  }
  out.info.visibleNodes = vis.length;

  // 1. 横向溢出：任何比视口宽的可滚动容器都要能滚，撑破页面的不行
  //    ⚠️ 两种祖先都要跳过，否则会把「根本没问题的布局」报成 high：
  //
  //    (a) 横向**可滚**祖先 —— 实测踩过：
  //        表格 TD/TH 全报「超出视口 53px」，但那张表 288px 放在 x=32
  //        （右边缘 320 < 359），且 overflow-x:auto、内容超 92px
  //        —— 那是**正确**的横向滚动。
  //
  //    (b) **裁剪**祖先 —— 实测踩过：
  //        工作区页打开时 DSH 渲染的 headerBlank / headerCorner 落在
  //        x=379 之后（视口 359），报 3 个 high。但它们被祖先 overflow
  //        裁掉，用户**根本看不见**，而页面宽仍是 359、无横向滚动。
  //        按画面逐条核对过，没有任何视觉破损。
  //
  //    漏掉这两步，就会去「修」一个根本不存在的问题。
  const ancestorHandles = (el) => {
    let n = el.parentElement;
    while (n && n !== document.body) {
      const cs = getComputedStyle(n);
      if (cs.overflowX === 'auto' || cs.overflowX === 'scroll') return 'scroll';
      if (cs.overflowX === 'hidden' || cs.overflowX === 'clip') return 'clip';
      n = n.parentElement;
    }
    return null;
  };
  for (const { el, cs, r } of vis) {
    if (ancestorHandles(el)) continue;
    if (r.right > VW + 2 && r.width <= VW + 2) {
      add('overflow-right', 'high',
          `元素右边缘超出视口 ${Math.round(r.right - VW)}px`, el);
    }
  }

  // 2. 触控目标 < 44dp（可点的：button / [role=button] / a / treeitem）
  //    ⚠️ **必须量「含 padding 的热区」，不能直接用 rect**。
  //    实测踩过两轮：
  //      · 行内链接靠纵向 padding 扩热区（padding 不改变行盒高度），
  //        而 getBoundingClientRect 对行内元素返回**内容盒** → 被扩到
  //        33px 热区的链接仍报 15px
  //      · `::after` 覆盖层扩出来的热区 rect 也完全看不见
  //    两类都是「量不出来」而不是「真的不合格」，照报只会让人去修错地方。
  const tap = 'button,[role=button],a,[role=treeitem],[role=tab]';
  let small = 0;
  for (const el of document.querySelectorAll(tap)) {
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden') continue;
    const r = el.getBoundingClientRect();
    if (r.width < 1 || r.height < 1) continue;
    const hitH = r.height
      + (parseFloat(cs.paddingTop) || 0)
      + (parseFloat(cs.paddingBottom) || 0);
    if (hitH < 44 || r.width < 32) {
      small++;
      if (small <= 8) add('tap-target', 'mid',
          `热区 ${Math.round(r.width)}×${Math.round(hitH)} < 44/32`, el);
    }
  }
  out.info.smallTapTargets = small;

  // 3. 文案被省略号砍掉
  //    ⚠️ 只看 css 有没有 ellipsis 会误报一堆 —— 设了 ellipsis 但内容
  //    本来就放得下的不算截断。判据必须是 scrollWidth > clientWidth。
  let trunc = 0;
  for (const { el, cs } of vis) {
    if (cs.textOverflow !== 'ellipsis') continue;
    const really = el.scrollWidth > el.clientWidth + 1;
    if (!really) continue;
    const r = el.getBoundingClientRect();
    if (r.width < 1) continue;
    const t = (el.innerText || '').trim();
    if (!t) continue;
    trunc++;
    if (trunc <= 8) {
      const hidden = el.scrollWidth - el.clientWidth;
      add('truncated', 'mid',
          `少了 ${hidden}px：「${t.slice(0, 22)}」`, el);
    }
  }
  out.info.truncated = trunc;

  // 4. 正文字号 < 12px
  let tiny = 0;
  for (const { el, cs } of vis) {
    const fs = parseFloat(cs.fontSize);
    if (fs && fs < 12 && (el.innerText || '').trim()) {
      tiny++;
      if (tiny <= 6) add('tiny-font', 'mid', `字号 ${fs}px < 12px`, el);
    }
  }
  out.info.tinyFont = tiny;

  // 5. 关键区域的高度占用（顶栏是不是吃掉太多）
  const header = document.querySelector('[data-slot="conversation.header"] header')
               || document.querySelector('header');
  if (header) {
    const r = header.getBoundingClientRect();
    out.info.headerH = Math.round(r.height);
    out.info.headerPct = Math.round((r.height / VH) * 100);
    if (r.height / VH > 0.22) {
      add('header-too-tall', 'mid',
          `顶栏 ${Math.round(r.height)}px 占屏高 ${out.info.headerPct}%`, header);
    }
  }
  const frame = document.querySelector('[data-slot="root"] > *');
  if (frame) {
    const cs = getComputedStyle(frame);
    out.info.frameCols = cs.gridTemplateColumns;
    out.info.collapsed = frame.hasAttribute('data-sidebar-collapsed');
  }
  const side = document.querySelector('[data-slot="sidebar"] > *');
  if (side) {
    const cs = getComputedStyle(side);
    const r = side.getBoundingClientRect();
    out.info.sidebar = { pos: cs.position, w: Math.round(r.width), vis: cs.visibility };
  }
  out.info.menuBtn = !!document.querySelector('.qx-menu-btn');
  out.info.treeItems = document.querySelectorAll('[role="treeitem"]').length;
  out.info.qx = !!window.qxMobile;

  out.sev = { high: 0, mid: 0 };
  for (const p of out.problems) out.sev[p.sev]++;
  return JSON.stringify(out);
})()
"""


def run() -> dict:
    tmp = ROOT / ".tmp" / "_audit.js"
    tmp.write_text(JS, encoding="utf-8")
    cmd = [sys.executable, str(CDP)]
    # 支持 --serial <序列号>，这样同一套审查既能跑真机也能跑模拟器。
    # 不加的话默认 emulator-5554 —— 而模拟器很容易被调成与真机一致，
    # 很容易让人忘了自己其实在看替身。
    if "--serial" in sys.argv:
        i = sys.argv.index("--serial")
        cmd += ["--serial", sys.argv[i + 1]]
    cmd += ["eval", "--file", str(tmp)]
    proc = subprocess.run(
        cmd, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=300,
    )
    tail = [ln for ln in proc.stdout.strip().splitlines() if ln.strip().startswith("{")]
    if not tail:
        raise SystemExit(f"没拿到结果：\n{proc.stdout[-800:]}\n{proc.stderr[-400:]}")
    return json.loads(tail[-1])


def report(d: dict) -> None:
    v = d["viewport"]
    print(f"视口 {v['w']}×{v['h']} dpr={v['dpr']}   页面宽 {d['docScrollW']}"
          f"{'  🔴 横向溢出' if d['overflowX'] else '  ✓ 无横向溢出'}")
    i = d["info"]
    print(f"可见节点 {i.get('visibleNodes')}  触控过小 {i.get('smallTapTargets')}  "
          f"截断 {i.get('truncated')}  小字号 {i.get('tinyFont')}")
    print(f"顶栏 {i.get('headerH')}px（{i.get('headerPct')}% 屏高）  "
          f"栅格 {i.get('frameCols')}  折叠 {i.get('collapsed')}")
    print(f"侧栏 {i.get('sidebar')}  ☰ {i.get('menuBtn')}  "
          f"树项 {i.get('treeItems')}  定制层 {i.get('qx')}")
    sev = d["sev"]
    print(f"\n问题 {len(d['problems'])} 个（high {sev['high']} / mid {sev['mid']}）")
    for p in d["problems"]:
        print(f"  [{p['sev']:<4}] {p['kind']:<16} {p['msg']:<44} {p['box']} {p['tag']}")


def main() -> int:
    d = run()
    if "--json" in sys.argv:
        print(json.dumps(d, ensure_ascii=False, indent=2))
    else:
        report(d)
    # 有 high 就以非 0 退出，方便脚本判断
    return 1 if d["sev"]["high"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
