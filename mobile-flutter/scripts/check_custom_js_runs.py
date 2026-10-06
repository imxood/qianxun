"""在部署**之前**跑一遍 custom.js，验它真的会执行。

为什么必须有这道闸
----------------
上一版 custom.js 的教训：语法完全合法、文件被网关正常服务、hash 正确更新，
但**界面一点反应都没有**。根因是入口写成了立即执行的 async 箭头，
`register` 拿到 Promise 而非函数，于是 mount 从未被调用。

而 `node --check` **查不出**这个问题 —— 它只验语法，不验「返回的东西
对不对形状」。这类失败在真机上的表现是「什么都没有」，连报错都没有
（bootstrap 只 catch 同步异常）。

所以这里用一个**最小 DOM 替身**跑一遍，检查三件事：
  1. `window.qxMobile.register` 真的被调用
  2. 传进去的 mount **是一个函数**
  3. 调用它之后，☰ 按钮真的被插进 document.body
  4. 它返回的是**函数**（清理用），不是 Promise

用法：python scripts/check_custom_js_runs.py
"""
from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

TARGET = Path(
    r"E:\develop\dsh-workspace\qianxun\mobile-flutter\qx-mobile\custom.js"
)

# 最小 DOM 替身。够 custom.js 跑起来，不多不少。
HARNESS = r"""
const fs = require('fs');
const src = fs.readFileSync(process.argv[2], 'utf8');

const made = [];
const pendingTimeouts = [];   // 捕获 setTimeout，闸门要手动跑
function makeStyle() {
  const s = { map: {}, setProperty(k, v) { s.map[k] = v; }, removeProperty(k) { delete s.map[k]; } };
  return s;
}
function makeEl(tag) {
  const node = {
    tagName: String(tag).toUpperCase(),
    nodeType: 1,
    style: makeStyle(),
    dataset: {},
    className: '',
    children: [],
    attrs: {},
    isConnected: true,
    hidden: false,
    _text: '',
    // 真 DOM 的 textContent 会**向下聚合**子节点文本。
    // 必须在这里也聚合，否则「找含文案的最小容器」这类逻辑测不出真问题
    // —— 第 3 版之前的弹窗抑制就是被这一点掩盖掉的。
    get textContent() {
      const own = this._text || '';
      if (!this.children || !this.children.length) return own;
      return own + this.children.map((c) => c.textContent).join('');
    },
    set textContent(v) { this._text = String(v); },
    // 真 DOM 里元素的 parentElement === parentNode
    get parentElement() { return this.parentNode && this.parentNode.nodeType === 1 ? this.parentNode : null; },
    setAttribute(k, v) { this.attrs[k] = v; if (k.startsWith('data-')) this.dataset[k.slice(5)] = v; },
    getAttribute(k) { return this.attrs[k] === undefined ? null : this.attrs[k]; },
    removeAttribute(k) { delete this.attrs[k]; },
    hasAttribute(k) { return this.attrs[k] !== undefined; },
    // custom.js 用 `.append(...)`（不是 appendChild）往 layer 里塞东西
    append(...cs) { for (const c of cs) { if (c) { this.children.push(c); c.parentNode = this; } } },
    appendChild(c) { if (c) { this.children.push(c); c.parentNode = this; } return c; },
    prepend(c) { if (c) this.children.unshift(c); },
    replaceChildren(...cs) { this.children = cs; },
    remove() { this.isConnected = false; if (this.parentNode) this.parentNode.children = this.parentNode.children.filter(x => x !== this); },
    addEventListener(k, fn) { (this.__l || (this.__l = {}))[k] = fn; },
    removeEventListener(k) { if (this.__l) delete this.__l[k]; },
    dispatch(k) { if (this.__l && this.__l[k]) this.__l[k]({ target: this }); },
    click() { this.dispatch('click'); },
    closest(sel) {
      // 只支持 '[role="treeitem"]' 这种单属性选择器 —— custom.js 就只用这一种
      const m = /^\[([^=\]]+)="([^"]*)"\]$/.exec(sel);
      if (!m) return null;
      let n = this;
      while (n) {
        if (n.getAttribute && n.getAttribute(m[1]) === m[2]) return n;
        n = n.parentElement;
      }
      return null;
    },
    querySelector() { return null; },
    // 支持 `.class` 选择器 —— appendHint 靠它清旧提示条，
    // 不支持的话闸门会**永远看不到重复**，等于这条断言是假的。
    querySelectorAll(sel) {
      if (typeof sel !== 'string' || sel[0] !== '.') return [];
      const want = sel.slice(1);
      const out = [];
      const walk = (n) => {
        for (const c of (n.children || [])) {
          if (c.className && c.className.split(' ').indexOf(want) !== -1) out.push(c);
          walk(c);
        }
      };
      walk(this);
      return out;
    },
    contains() { return false; },
    cloneNode() { return makeEl(tag); },
    get firstChild() { return this.children[0] || null; },
  };
  made.push(node);
  return node;
}
const body = makeEl('body');
const document = Object.assign(makeEl('document'), {
  body,
  visibilityState: 'visible',
  createElement: makeEl,
  createTextNode: (t) => ({ textContent: t, nodeType: 3 }),
  querySelectorAll: () => [],
  // ⚠️ 不要用 makeEl 的实现 —— 下面 Object.assign 会覆盖它，
  //    所以这里必须**显式**给一组能捕获的实现，否则
  //    「选完会话自动收起」这条根本触发不了（闸门会永远通过）。
  addEventListener(k, fn) { (this.__l || (this.__l = {}))[k] = fn; },
  removeEventListener(k) { if (this.__l) delete this.__l[k]; },
});

let registered = null;
let tick = null;   // 捕获 setInterval 的回调，闸门要主动触发它

// ── 假的 session/list 返回 ────────────────────────────────────────────
// ⚠️ **故意放两条标题完全相同的会话**（真机实测：394 条里有 4 条这样）。
//    第 3 版按标题匹配，这两条会被认成同一个 —— 闸门就是用来钉死这个缺陷的。
const SAME_TITLE = '我一般不看新闻 论坛 社交媒';
const FAKE_ITEMS = [
  { sessionId: 'session-AAA', updatedAt: 200, running: false,
    projections: { values: { title: SAME_TITLE } } },                    // 稀疏
  { sessionId: 'session-BBB', updatedAt: 100, running: false,
    projections: { values: { title: SAME_TITLE, turnOutline: [{}],
      contextPressure: { projectedTokens: 60, contextWindow: 100 } } } }, // 上下文 60%
];

const window = {
  document,
  location: { pathname: '/', href: 'http://x/' },
  history: { pushState(){}, replaceState(){} },
  console,
  innerWidth: 400,          // 手机宽度 —— 用来触发「选完自动收起」
  setTimeout: (fn) => { pendingTimeouts.push(fn); return pendingTimeouts.length; },
  clearTimeout() {},
  setInterval: (fn) => { tick = fn; return 1; },
  clearInterval() {},
  addEventListener() {},
  removeEventListener() {},
  dispatchEvent() {},
  getComputedStyle: (el) => ({ position: el && el.__pos ? el.__pos : 'static' }),
  fetch: () => Promise.resolve({
    ok: true,
    json: () => Promise.resolve({ result: { ok: true, value: { items: FAKE_ITEMS } } }),
  }),
  PopStateEvent: function () {},
  Event: function () {},
};

const qxMobile = { register(m) { registered = m; } };
// ⚠️ 必须挂在 window 上 —— custom.js 写的是 `window.qxMobile.register`，
//    挂成全局变量的话，sandbox 里那个 window 替身没有这个属性。
window.qxMobile = qxMobile;

// 用 new Function 而不是 eval：避开模块作用域，且能拿到返回值
let fn;
try {
  fn = new Function('window', 'document', 'navigator', src + '\n');
  fn(window, document, { clipboard: null });
} catch (e) {
  console.log('FAIL 执行抛错: ' + e.message);
  process.exit(1);
}

if (registered === null) {
  console.log('FAIL window.qxMobile.register 从未被调用');
  process.exit(1);
}
if (typeof registered !== 'function') {
  console.log('FAIL register 收到的不是函数，而是 ' + typeof registered);
  console.log('     这是「语法合法但界面完全不动」的那类静默失败');
  process.exit(1);
}
console.log('PASS register 收到的是函数');

(async () => {
  // 窄屏导航用的锚点必须在 mount **之前**建好 ——
  // installMenuButton() 是在 mount 里跑的，锚点晚建就断言不到。
  // 实测：顶栏左侧那个槽是空的（display:contents、innerHTML ""）。
  const lead = makeEl('div');
  lead.setAttribute('data-slot', 'conversation.header.leading');
  const openBtn = makeEl('button');
  openBtn.setAttribute('aria-label', 'Open sidebar');
  let opened = 0;
  openBtn.addEventListener('click', () => { opened += 1; });
  const frame = makeEl('div');
  frame.setAttribute('data-slot', 'root');
  document.querySelector = (q) => {
    if (q === '[data-slot="conversation.header.leading"]') return lead;
    if (q === '[data-slot="root"] > *') return frame;
    return null;
  };

  let dispose;
  try {
    dispose = registered({ document, window, root: makeEl('div') });
  } catch (e) {
    console.log('FAIL mount 调用抛错: ' + e.message);
    process.exit(1);
  }
  // mount 是 async，返回 Promise 是**正常**的；
  // 关键是这个 Promise resolve 后**返回的**东西必须是函数。
  if (dispose && typeof dispose.then === 'function') {
    try { dispose = await dispose; }
    catch (e) {
      console.log('FAIL mount 异步体抛错: ' + e.message);
      process.exit(1);
    }
  }
  if (typeof dispose !== 'function') {
    console.log('FAIL mount 返回的不是清理函数，而是 ' + typeof dispose);
    process.exit(1);
  }
  console.log('PASS mount 返回清理函数');

  // ── 第 3 版起：☰ 抽屉已撤掉（与 DSH 自带 rail 撞成两套导航）。
  //    所以「按钮在不在」不再是检查项，改成「**不该再有**」。
  //    这条正是在第 2 版之后才加的：闸门本该拦的正是「
  //    界面完全不动」这类静默失败。
  let toggle = null;
  let scan = (node) => {
    for (const c of (node.children || [])) {
      if (c && c.className === 'qx-toggle') toggle = c;
      scan(c);
    }
  };
  scan(body);
  if (toggle) {
    console.log('FAIL 页面里还有 qx-toggle —— 第 3 版应当已撤掉第二套导航');
    process.exit(1);
  }
  console.log('PASS 没有 qx-toggle（第二套导航已撤掉）');

  // 反过来：层必须在（工具卡 / 侧栏加工要靠它）
  let layer = null;
  let scan2 = (node) => {
    for (const c of (node.children || [])) {
      if (c && c.className === 'qx-layer') layer = c;
      scan2(c);
    }
  };
  scan2(body);
  if (!layer) {
    console.log('FAIL qx-layer 没有插入 document.body');
    process.exit(1);
  }
  if (layer.style.zIndex !== '2147483644') {
    console.log('FAIL qx-layer zIndex 不是 2147483644（实际 ' + layer.style.zIndex + '）');
    process.exit(1);
  }
  console.log('PASS qx-layer 已插入且 zIndex 正确');

  // ── 弹窗抑制：必须用**真机实测到的结构**做替身，否则测不出真问题。
  //
  //   真机结构（cdp_webview.py 实测，2026-xx）：
  //     DIV._root_o6lrb_6            ← 通用 Dialog 宿主（mask + panel）
  //       ├── DIV._mask_o6lrb_18      position:absolute，**文案为空**，吞点击
  //       └── DIV._dialog_o6lrb_33   position:relative, z=1，[role=dialog]
  //            └── DIV
  //                 └── P           ← 真正的文案在这里
  //
  //   第 3 版之前的写法在 `div,section,aside,dialog` 里找最小容器、
  //   遇到「子节点也含文案」就跳过 —— 而含文案的叶子是 `<p>`，
  //   不在选择器里，于是 3 个候选全被跳过，**一个都没隐藏**。
  //   真机症状：hidden=0、弹窗照旧，像「代码没执行」。
  if (!tick) {
    console.log('FAIL 没有捕获到轮询回调 —— 弹窗抑制在真机上不会被调用');
    process.exit(1);
  }
  const root = makeEl('div');       root.className = '_root_o6lrb_6';
  const mask = makeEl('div');       mask.className = '_mask_o6lrb_18';   mask.__pos = 'absolute';
  const dialog = makeEl('div');     dialog.className = '_dialog_o6lrb_33'; dialog.__pos = 'relative';
  const inner = makeEl('div');
  const para = makeEl('p');
  para.textContent = 'DeepSeek Harness 0.2 is still in preview';
  inner.children = [para]; para.parentNode = inner;
  dialog.children = [inner]; inner.parentNode = dialog;
  dialog.setAttribute('role', 'dialog');
  root.children = [mask, dialog]; mask.parentNode = root; dialog.parentNode = root;
  // 顺带放一个**不该被隐藏**的弹窗：审批/提问卡片也走同一个宿主
  const approval = makeEl('div'); approval.__pos = 'relative';
  approval.setAttribute('role', 'dialog');
  approval.textContent = '是否允许执行 bash？';
  root.children.push(approval); approval.parentNode = root;
  body.children = [root];

  // ⚠️ 选择器分派必须**区分**「标签列表」和「属性选择器」，否则闸门是假的。
  //    真机上第 3 版之所以一条都没命中，正是因为它在 `div,section,aside,dialog`
  //    里找最小容器：命中的是 root/dialog/inner 三层 div，每层都有
  //    「子节点也含文案」的子节点，于是全被跳过，而真正的文案在 `<p>` 里 ——
  //    `<p>` 不在那个标签列表里，永远轮不到。
  //    之前这里只要看到 'dialog' 字样就返回弹窗，等于替旧写法**兜了底**，
  //    变异测试（.tmp/mutate_gate.py）当场报「闸门漏了」。
  const TAGS = /(^|[, ])div([, ]|$)/;
  const sel = {
    divChain() { return [root, mask, dialog, inner]; },
    dialogs() { return [dialog, approval]; },
    tree() { return treeRows; },
    sidebarButtons() { return [collapseBtn]; },
  };
  document.querySelectorAll = (q) => {
    if (q.indexOf('treeitem') !== -1) return sel.tree();
    if (q.indexOf('[role="dialog"]') !== -1 || q.indexOf('[aria-modal') !== -1) return sel.dialogs();
    if (q.indexOf('sidebar') !== -1) return sel.sidebarButtons();
    if (TAGS.test(q)) return sel.divChain();
    return [];
  };

  try { tick(); } catch (e) {
    console.log('FAIL 轮询回调抛错: ' + e.message);
    process.exit(1);
  }
  if (dialog.style.map.display !== 'none') {
    console.log('FAIL Preview Notice 面板没有被隐藏（display=' + dialog.style.map.display + '）');
    process.exit(1);
  }
  if (mask.style.map.display !== 'none') {
    console.log('FAIL 遮罩没有被隐藏 —— 面板没了但底下照样点不动');
    process.exit(1);
  }
  if (approval.style.map.display === 'none') {
    console.log('FAIL 审批弹窗被误伤 —— 抑制范围必须只限文案匹配的那一个');
    process.exit(1);
  }
  console.log('PASS Preview Notice 与遮罩都隐藏，且没误伤审批弹窗');

  // ── 第 4 版：data-row-key **精确**匹配 ────────────────────────────
  // 造 4 行：两条**标题完全相同**的会话、一条列表里没有的、一条工作区行。
  // 第 3 版按标题匹配 → 前两行会被认成同一条（取最近更新的那条），
  // 于是 BBB 拿到的是 AAA 的摘要（稀疏提示），上下文 60% 就不见了。
  const rowAAA = makeEl('div');
  rowAAA.setAttribute('role', 'treeitem');
  rowAAA.setAttribute('data-row-key', 'session:session-AAA');
  rowAAA.textContent = SAME_TITLE;

  const rowBBB = makeEl('div');
  rowBBB.setAttribute('role', 'treeitem');
  rowBBB.setAttribute('data-row-key', 'session:session-BBB');
  rowBBB.textContent = SAME_TITLE;          // ← 与 AAA 一字不差

  const rowZZZ = makeEl('div');
  rowZZZ.setAttribute('role', 'treeitem');
  rowZZZ.setAttribute('data-row-key', 'session:session-ZZZ');
  rowZZZ.textContent = SAME_TITLE;

  const rowWs = makeEl('div');
  rowWs.setAttribute('role', 'treeitem');
  rowWs.setAttribute('data-row-key', 'workspace:48231145-4874-434e-b47c-06070bff3668');
  rowWs.textContent = '服务部署';

  const treeRows = [rowAAA, rowBBB, rowZZZ, rowWs];

  tick(); tick();
  const kids = (n) => n.children.map((c) => c.className).join(',');
  if (kids(rowAAA).indexOf('qx-hint-sparse') === -1) {
    console.log('FAIL session-AAA 没拿到稀疏提示（实际子节点: ' + kids(rowAAA) + '）');
    process.exit(1);
  }
  if (kids(rowBBB).indexOf('qx-hint-sparse') !== -1) {
    console.log('FAIL session-BBB 被当成稀疏行 —— 同名会话被认错了');
    process.exit(1);
  }
  if (kids(rowBBB).indexOf('qx-hint-ctx') === -1) {
    console.log('FAIL session-BBB 没有「上下文 60%」（同名会话被认错了）');
    process.exit(1);
  }
  if (kids(rowZZZ).length > 0) {
    console.log('FAIL 列表里没有的 session-ZZZ 被加工了（' + kids(rowZZZ) + '）');
    process.exit(1);
  }
  if (kids(rowWs).length > 0) {
    console.log('FAIL 工作区行被当会话加工了');
    process.exit(1);
  }
  console.log('PASS 按 data-row-key 精确匹配：同名两条各自拿到自己的摘要');

  // ── 第 4 版：选完会话自动收起侧栏 ────────────────────────────────
  // 真机症状：点会话会跳转，但侧栏不收起 → 浮层挡住，永远看不到对话。
  const collapseBtn = makeEl('button');
  collapseBtn.setAttribute('aria-label', 'Collapse sidebar');
  // 真实 DSH 里点这个按钮会收起侧栏；替身要**真的产生状态变化**，
  // 否则断言只能验「按钮被点了」，验不到「侧栏真收起了」。
  collapseBtn.addEventListener('click', () => frame.setAttribute('data-sidebar-collapsed', ''));
  sel.sidebarButtons = () => [collapseBtn];

  const onClick = document.__l && document.__l.click;
  if (!onClick) {
    console.log('FAIL 没装上「选完自动收起」的监听');
    process.exit(1);
  }
  onClick({ target: rowBBB });                      // 点一个**会话**行
  for (const fn of pendingTimeouts.splice(0)) fn();
  if (!frame.hasAttribute('data-sidebar-collapsed')) {
    console.log('FAIL 点会话后侧栏没收起 —— 浮层会一直挡住对话');
    process.exit(1);
  }

  // 反过来：点**工作区**行不该收起（那是展开/折叠，不是选会话）
  frame.removeAttribute('data-sidebar-collapsed');
  onClick({ target: rowWs });
  for (const fn of pendingTimeouts.splice(0)) fn();
  if (frame.hasAttribute('data-sidebar-collapsed')) {
    console.log('FAIL 点工作区行也把侧栏收起了');
    process.exit(1);
  }
  console.log('PASS 点会话后自动收起，点工作区行不收起');

  // ── 第 5 版：窄屏 ☰ 打开工作区独立页 ─────────────────────────────
  // 真机实测的落点：[data-slot=conversation.header.leading]
  //   display:contents、**内容为空**（innerHTML ""）、位置是顶栏最左 0×48 的空位。
  // ⚠️ 窄屏下 rail 是 display:none，但 display:none 元素上的 .click()
  //    **照样派发** —— 所以「打开工作区」不需要手改 DSH 的状态机。
  sel.sidebarButtons = () => [openBtn];

  const mountedMenu = lead.children.filter((c) => c.className === 'qx-menu-btn');
  if (mountedMenu.length !== 1) {
    console.log('FAIL 顶栏空槽里没有且仅有一个 qx-menu-btn（实得 ' + mountedMenu.length + '）');
    process.exit(1);
  }
  if (lead.children.length !== 1) {
    console.log('FAIL 菜单按钮挤掉了槽里原有的东西');
    process.exit(1);
  }
  mountedMenu[0].click();
  if (opened !== 1) {
    console.log('FAIL 点 ☰ 没有触发 DSH 自己的「Open sidebar」（opened=' + opened + '）');
    process.exit(1);
  }
  console.log('PASS ☰ 落在顶栏空槽，点击走 DSH 自己的 Open sidebar');

  // ── 提示条不能重复（必须放在最后：它会 dispose 掉当前这一轮）──────
  // ⚠️ 真机症状：定制层热替换后，DSH 行里出现**两个**「上下文 79%」。
  //   原因：提示条插在 DSH 自己的行里（不在我们的 layer 里），
  //   dispose 掉 layer 带不走它们；新 mount 的 state.painted 又是空的。
  // 这里真跑一次「dispose → 再 mount」，复现整条热替换路径。
  const countCtx = (r) => r.children.filter(
    (c) => c.className.indexOf('qx-hint-ctx') === 0).length;
  const before = treeRows.map(countCtx);
  // 注意：不是每行都该有提示 —— AAA 是稀疏行（本就没有），
  // ZZZ 不在列表里、Ws 是工作区行。所以只查「没超过 1」。
  if (before.some((n) => n > 1)) {
    console.log('FAIL 第一轮就有重复：' + JSON.stringify(before));
    process.exit(1);
  }
  if (before.filter((n) => n === 1).length !== 1) {
    console.log('FAIL 第一轮该有一条上下文提示（BBB），实得 ' + JSON.stringify(before));
    process.exit(1);
  }

  dispose();                                   // 模拟 bootstrap 的热替换
  let dispose2;
  try {
    dispose2 = registered({ document, window, root: makeEl('div') });
    if (dispose2 && typeof dispose2.then === 'function') dispose2 = await dispose2;
  } catch (e) {
    console.log('FAIL 第二次 mount 抛错: ' + e.message);
    process.exit(1);
  }
  tick(); tick();
  const after = treeRows.map(countCtx);
  if (after.some((n, i) => n > before[i])) {
    console.log('FAIL 热替换后提示条变多了：' + JSON.stringify(before) + ' → ' + JSON.stringify(after));
    process.exit(1);
  }
  if (after.some((n) => n > 1)) {
    console.log('FAIL 热替换后出现重复：' + JSON.stringify(after));
    process.exit(1);
  }
  console.log('PASS 提示条幂等：热替换（dispose → 再 mount）后仍只有一条');
  dispose2();
  console.log('PASS dispose 可调用');
  console.log('\n全部通过 ✓');
})();
"""


def main() -> int:
    # 可选：指定被测文件。用来拿「故意改坏的版本」验证闸门**抓得住** ——
    # 一道从没红过的闸门不能证明它有用。
    target = Path(sys.argv[1]) if len(sys.argv) > 1 else TARGET
    if not target.is_file():
        print(f"找不到 {target}")
        return 1

    # 第一关：语法
    proc = subprocess.run(
        ["node", "--check", str(target)], capture_output=True, text=True
    )
    if proc.returncode != 0:
        print("✗ 语法错误：")
        print(proc.stderr[:800])
        return 1
    print("✓ 语法通过")

    # 第二关：行为
    # ⚠️ 必须用 .cjs —— 仓库根 package.json 有 "type": "module"，
    #   .js 会被当 ESM，而 ESM 里 require 未定义。
    harness_path = Path(__file__).resolve().parent / "_check_harness.cjs"
    harness_path.write_text(HARNESS, encoding="utf-8")
    proc = subprocess.run(
        ["node", str(harness_path), str(target)],
        capture_output=True, text=True, timeout=60,
    )
    print(proc.stdout.strip())
    if proc.stderr.strip():
        print(proc.stderr.strip()[:600])
    harness_path.unlink(missing_ok=True)
    return proc.returncode


if __name__ == "__main__":
    raise SystemExit(main())
