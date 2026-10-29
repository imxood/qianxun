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
  // ⚠️ 必须记住 `important`。第 9 轮查明：消息操作按钮的 min-width
  //    **只能靠行内 !important 改**——行内 !important 在层叠里压过任何
  //    样式表规则（实测：把规则插进真表，盒子纹丝不动）。
  //    夹具以前直接丢掉第三个参数，于是「写没写 important」在测试里
  //    毫无差别 —— 改回普通行内声明这套闸门照样全绿。
  const s = {
    map: {},
    imp: {},
    setProperty(k, v, pri) { s.map[k] = v; if (pri) s.imp[k] = pri; },
    removeProperty(k) { delete s.map[k]; delete s.imp[k]; },
    get(k) { return s.map[k]; },
  };
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
      let n = this;
      while (n) {
        try { if (n.matches && n.matches(sel)) return n; } catch (e) { /* 不支持 */ }
        n = n.parentElement;
      }
      return null;
    },
    // ── 选择器引擎 ────────────────────────────────────────────────
    // ⚠️⚠️ 原来这里只认 `.class`，其余一律返回 [] / null。
    //    结果是：状态按钮、工具栏、统计区这些**新逻辑一行都没被执行到**，
    //    闸门却全绿 —— 正是 docs/design/15 §8 说的「绿得像没这回事」。
    //    现在支持 `.class` / `[attr]` / `[attr="v"]` / `tag` /
    //    空格后代 / `>` 直接子级 / 逗号并列。
    _matchesSimple(sel) {
      if (sel.charAt(0) === '.') {
        return String(this.className || '').split(' ').indexOf(sel.slice(1)) !== -1;
      }
      const am = /^\[([^=\]]+)(?:="([^"]*)")?\]$/.exec(sel);
      if (am) {
        if (!this.getAttribute) return false;
        if (am[2] === undefined) return this.hasAttribute(am[1]);
        return this.getAttribute(am[1]) === am[2];
      }
      return this.tagName === sel.toUpperCase();
    },
    // ⚠️ 只收 parts、用 this 当 el —— 写成 `_matchesChain(el, chain)` 再用
    //    `c._matchesChain(chain)` 调用的话，第一个参数被当成 el、
    //    chain 变 undefined，直接炸在 `chain.length` 上。
    // ⚠️ `_parseChain` 必须认 `>`：custom.js 有
    //    `document.querySelector('[data-slot="root"] > *')`
    //    （installAutoCollapseSidebar 用它找 frame）。
    //    不支持 `>` 的话 frame 恒为 null，「选完会话自动收起」那条会一直失败
    //    —— 看起来像 custom.js 坏了，其实是闸门的解析器。
    _parseChain(sel) {
      const toks = String(sel).trim().split(/\s+/);
      const out = [];
      let child = false;
      for (const t of toks) {
        if (t === '>') { child = true; continue; }
        out.push({ sel: t, child });
        child = false;
      }
      return out;
    },
    _matchesChain(parts) {
      const n = parts.length;
      let node = this;
      if (!node._matchesSimple(parts[n - 1].sel)) return false;
      for (let i = n - 2; i >= 0; i -= 1) {
        if (parts[i + 1].child) {
          node = node.parentElement;
          if (!node || !node._matchesSimple(parts[i].sel)) return false;
        } else {
          let p = node.parentElement;
          while (p && !p._matchesSimple(parts[i].sel)) p = p.parentElement;
          if (!p) return false;
          node = p;
        }
      }
      return true;
    },
    matches(sel) {
      const chains = String(sel).split(',').map((x) => this._parseChain(x)).filter((c) => c.length);
      return chains.some((ch) => ch.length && this._matchesChain(ch));
    },
    querySelector(sel) { return this.querySelectorAll(sel)[0] || null; },
    getBoundingClientRect() { return { x: 0, y: 0, width: 40, height: 20 }; },
    scrollWidth: 40,
    clientWidth: 40,
    querySelectorAll(sel) {
      const chains = String(sel).split(',').map((x) => this._parseChain(x)).filter((c) => c.length);
      if (!chains.length) return [];
      const out = [];
      const walk = (n) => {
        for (const c of (n.children || [])) {
          if (c._matchesSimple && chains.some((ch) => c._matchesChain(ch))) out.push(c);
          walk(c);
        }
      };
      walk(this);
      return out;
    },
    contains(other) {
      for (let n = other; n; n = n.parentElement) if (n === this) return true;
      return false;
    },
    insertBefore(node, ref) {
      if (!node) return node;
      if (!this.children.some((c) => c === ref)) return this.appendChild(node);
      const i = this.children.indexOf(ref);
      this.children.splice(i, 0, node);
      node.parentNode = this;
      return node;
    },
    cloneNode() { return makeEl(tag); },
    get firstChild() { return this.children[0] || null; },
    // 真 DOM 里 childNodes 含文本节点。替身给 children 就够用 ——
    // custom.js 只拿它把「图标 + 文字」拆成几段。
    // ⚠️ 不给这个属性，readStats 会抛 `node.childNodes is not iterable`，
    //    症状看起来像产品代码坏了，其实是替身不像浏览器。
    get childNodes() { return this.children; },
  };
  made.push(node);
  return node;
}
const body = makeEl('body');
const document = Object.assign(makeEl('document'), {
  body,
  documentElement: makeEl('html'),
  visibilityState: 'visible',
  createElement: makeEl,
  createTextNode: (t) => ({ textContent: t, nodeType: 3 }),
  // ⚠️ 必须**真的从 body 往下找**，不能用 `() => []`。
  //    custom.js 里 `document.querySelector('[data-slot="conversation.composer.bar"]')`
  //    这类调用非常多；一律返回 null 的话，状态按钮 / 工具栏 / 统计区
  //    三条新路径**全部静默跳过**，闸门照样全绿。
  querySelectorAll(sel) { return body.querySelectorAll(sel); },
  querySelector(sel) { return body.querySelector(sel); },
  // ⚠️ 不要用 makeEl 的实现 —— 下面 Object.assign 会覆盖它，
  //    所以这里必须**显式**给一组能捕获的实现，否则
  //    「选完会话自动收起」这条根本触发不了（闸门会永远通过）。
  addEventListener(k, fn) { (this.__l || (this.__l = {}))[k] = fn; },
  removeEventListener(k) { if (this.__l) delete this.__l[k]; },
});
document.documentElement.children = [body];
body.parentNode = document.documentElement;

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
  getComputedStyle: (el) => ({
    position: el && el.__pos ? el.__pos : 'static',
    display: (el && el.__disp) || 'flex',
    // 给足字号，好让 markTinyText 走「已达标」那条分支；
    // 越界分支另由下面的专属断言覆盖。
    fontSize: '16px',
    // ⚠️ 补齐 custom.js 会读的这些布局属性。
    //    真浏览器的 getComputedStyle 一定有它们，替身缺一个就会在
    //    `cs.flexDirection.indexOf(...)` 上抛 TypeError ——
    //    症状是「闸门报错」，很容易误判成 custom.js 坏了。
    //    **替身要像浏览器，不要逼产品代码写防御性判断。**
    flexDirection: 'row',
    flexWrap: 'nowrap',
    flex: '0 1 auto',
    justifyContent: 'flex-start',
    alignItems: 'center',
    gap: '8px',
    minWidth: 'auto',
    visibility: 'visible',
    overflow: 'visible',
    // ⚠️ 第 10 轮补：真浏览器的 getComputedStyle **永远**有 backgroundColor，
    //    没设背景的元素返回 `rgba(0, 0, 0, 0)`（不是 undefined）。
    //    缺这一项时 widenDotTaps 的「透明背景」判据拿到 undefined，
    //    正则匹配不上 → 走到默认值 1 → **所有元素都被当成不透明而跳过**，
    //    断言报「没补到 44」，看起来像产品代码坏了，其实是替身不完整。
    backgroundColor: 'rgba(0, 0, 0, 0)',
    width: '40px',
    height: '20px',
    lineHeight: 'normal',
    ...(el && el.__cs ? el.__cs : {}),
  }),
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
    // ⚠️ **必须回落**。原来这里 `return null`，等于 custom.js 里
    //    `document.querySelector('[data-slot="conversation.composer.bar"]')`
    //    永远拿不到东西 → 状态按钮 / 统计区三条新路径**一条都不执行**，
    //    闸门却照样全绿。真机上它们是能跑通的，所以这是纯粹的「测不到」。
    return body.querySelector(q);
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
  let layers = [];
  let scan2 = (node) => {
    for (const c of (node.children || [])) {
      if (c && c.className === 'qx-layer') { layer = c; layers.push(c); }
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
  console.log('PASS qx-layer 已插入、zIndex 正确');

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
    plain() { return sel.__plain || []; },
  };
  document.querySelectorAll = (q) => {
    if (q.indexOf('treeitem') !== -1) return sel.tree();
    if (q.indexOf('[role="dialog"]') !== -1 || q.indexOf('[aria-modal') !== -1) return sel.dialogs();
    if (q.indexOf('sidebar') !== -1) return sel.sidebarButtons();
    // ⚠️ 必须排在 TAGS 前面：'span, div' 里也有 div，
    //    走 divChain 分支的话 markClippedRows / markTinyText
    //    永远看不到下面造的那两个「坏元素」，闸门就绿得像没这回事。
    if (q.indexOf('span') !== -1 || q.indexOf('samp') !== -1) return sel.plain();
    if (TAGS.test(q)) return sel.divChain();
    // ⚠️ **必须回落到底层的选择器引擎**，不能 `return []`。
    //    这一组替身只负责提供「合成元素」（弹窗 / 侧栏树 / 侧栏按钮），
    //    它们在假 DOM 里本来就不存在；
    //    一旦在这里截断，后面新加的选择器（状态区、工具栏…）
    //    就**一条都匹配不到**，闸门照样全绿 —— 这正是 §8 记的那个坑。
    return body.querySelectorAll(q);
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
  // ⚠️ 上下文标签**已按用户要求从列表移除**（重复信息，
  //    且它 position:absolute 与标题重叠 52px）。
  //    这里反过来断言它**不会**再出现 —— 用户明确提过的要求，
  //    值得钉一条断言，否则很容易在重构时又插回来。
  for (const r of treeRows) {
    if (kids(r).indexOf('qx-hint-ctx') !== -1) {
      console.log('FAIL 列表里又出现了上下文标签（' + r.getAttribute('data-row-key')
        + '）—— 它已改由 composer 工具栏的「N%」标签承担');
      process.exit(1);
    }
  }
  console.log('PASS 列表里不再插上下文标签（改由 composer 工具栏的「N%」承担）');
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

  // ── 侧栏切换按钮：**中文** aria-label（真机上的真实值）──────────────
  // ⚠️ 这条是补上一个真机才暴露的 bug：模拟器上 DSH 是英文，
  //    `Open sidebar` 能匹配上，所以闸门一直是绿的；
  //    真机上同一个按钮叫「打开侧边栏」，点 ☰ **完全没反应**，
  //    而界面上看不出任何异常。
  //    所以闸门必须用中文 label 跑一遍，否则这个 bug 还会再犯。
  const zhOpen = makeEl('button');
  zhOpen.setAttribute('aria-label', '打开侧边栏');
  let zhClicked = 0;
  zhOpen.addEventListener('click', () => { zhClicked += 1; });
  sel.sidebarButtons = () => [zhOpen];
  mountedMenu[0].click();
  if (zhClicked !== 1) {
    console.log('FAIL 中文 aria-label「打开侧边栏」的按钮没被点到（zhClicked=' + zhClicked + '）');
    process.exit(1);
  }
  console.log('PASS 中文 aria-label「打开侧边栏」也能点到（模拟器英文、真机中文）');

  const zhCollapse = makeEl('button');
  zhCollapse.setAttribute('aria-label', '收起侧边栏');
  let zhCollapsed = 0;
  zhCollapse.addEventListener('click', () => { zhCollapsed += 1; });
  sel.sidebarButtons = () => [zhCollapse];
  frame.removeAttribute('data-sidebar-collapsed');       // 展开态
  const onClick2 = document.__l && document.__l.click;
  if (!onClick2) { console.log('FAIL 监听被摘掉了'); process.exit(1); }
  onClick2({ target: rowBBB });
  for (const fn of pendingTimeouts.splice(0)) fn();
  if (zhCollapsed !== 1) {
    console.log('FAIL 中文「收起侧边栏」没有被点到（zhCollapsed=' + zhCollapsed + '）');
    process.exit(1);
  }
  console.log('PASS 中文「收起侧边栏」也能点到（选完会话自动收起）');

  // ── 窄屏排版两道标记 ──────────────────────────────────────────────
  // 这两道是「按行为找元素」，所以替身必须能造出对应的坏元素，
  // 否则闸门永远走「一切正常」那条分支 —— 绿得像没这回事。
  const tinyEl = makeEl('span');
  tinyEl.__disp = 'block';
  tinyEl.children = [{ nodeType: 3, textContent: 'x' }];
  // ⚠️ 不能写 `tinyEl.firstChild = {...}` —— makeEl 的 firstChild 只有 getter，
  //    赋值会被**静默忽略**，于是 `!el.firstChild` 恒成立、
  //    markTinyText 每次都 continue，闸门就绿得像没这回事。
  const tinyBox = {
    width: 40, height: 20, x: 0, y: 0,
    scrollWidth: 40, clientWidth: 40,
  };
  tinyEl.getBoundingClientRect = () => tinyBox;
  tinyEl.scrollWidth = 40;
  tinyEl.clientWidth = 40;

  const clippedEl = makeEl('span');
  const clippedBox = {
    width: 40, height: 20, x: 0, y: 0,
    scrollWidth: 103, clientWidth: 40,     // ← 真的被砍了 63px
  };
  clippedEl.getBoundingClientRect = () => clippedBox;
  clippedEl.scrollWidth = 103;
  clippedEl.clientWidth = 40;
  clippedEl.children = [{ nodeType: 3, textContent: 'y' }];

  const row = makeEl('div');
  row.children = [clippedEl];
  clippedEl.parentNode = row;
  row.getBoundingClientRect = () => ({ width: 200, height: 20, x: 0, y: 0 });
  body.children = [row];

  // 让 getComputedStyle 对这两个元素报 11px
  const prevGCS = window.getComputedStyle;
  sel.__plain = [tinyEl, clippedEl];
  window.getComputedStyle = (el) => {
    if (el === tinyEl || el === clippedEl) {
      return { position: 'static', display: 'block', fontSize: '11px' };
    }
    return prevGCS(el);
  };
  tick();

  if (!clippedEl.attrs['data-qx-clipped'] && !row.attrs['data-qx-clipped']) {
    console.log('FAIL 被截断的行没有被打上 data-qx-clipped');
    process.exit(1);
  }
  if (!tinyEl.attrs['data-qx-tiny']) {
    console.log('FAIL 11px 的文本没有被打上 data-qx-tiny');
    process.exit(1);
  }
  console.log('PASS 被截断的行与小字号文本都被标记（CSS 据此改成换行 / 托到 12px）');
  window.getComputedStyle = prevGCS;

  // ── 第 6 版：状态条收进工具栏按钮 ─────────────────────────────────
  // ⚠️ 用**真值**构造：层级照 cdp_webview 在真机实测的结构摆。
  //    不做简化 —— 简化过的结构测不出「层级多一层」这类问题
  //    （markStatusRow 就因为数错了层级，连续三版都没标对）。
  //
  // 真机结构：
  //   [slot=conversation.composer.bar] (display:contents)
  //     DIV root
  //       DIV card                     ← 输入卡片（含工具栏行）
  //         DIV row
  //           DIV tools  > BUTTON「Add files」, DIV modes
  //                                [slot=conversation.input.left]
  //           DIV trailing > [slot=conversation.input.model]
  //                                 [slot=conversation.input.right], BUTTON Send
  //       DIV dock                    ← 统计区（要被收起来的）
  //         DIV pills > SPAN > BUTTON「11 turns…」
  //                    SPAN > BUTTON「89.2M tok…」
  //         SPAN ring > BUTTON「79%」
  const mk = (tag, cls) => { const e = makeEl(tag); if (cls) e.className = cls; return e; };
  const cbar = mk('div'); cbar.setAttribute('data-slot', 'conversation.composer.bar');
  const compRoot = mk('div', 'root');
  const card = mk('div', 'card');
  const trow = mk('div', 'row');
  const tools = mk('div', 'tools');
  const addBtn = mk('button'); addBtn.setAttribute('aria-label', 'Add files or resources');
  const modes = mk('div', 'modes');
  const leftSlot = mk('div'); leftSlot.setAttribute('data-slot', 'conversation.input.left');
  const trailing = mk('div', 'trailing');
  const modelSlot = mk('div'); modelSlot.setAttribute('data-slot', 'conversation.input.model');
  const rightSlot = mk('div'); rightSlot.setAttribute('data-slot', 'conversation.input.right');
  const sendBtn = mk('button'); sendBtn.setAttribute('aria-label', 'Send');
  tools.append(addBtn, modes); modes.append(leftSlot);
  trailing.append(modelSlot, rightSlot, sendBtn);
  trow.append(tools, trailing);
  card.append(trow);
  // ⚠️ 替身的 getBoundingClientRect 默认 40×20，而 findToolbarRow 的判据之一是
  //    「行宽 ≥ 视口 70%」（真机实测 320/359）。不喂真实宽度的话
  //    那一行永远认不出来，闸门会「因为找不到行」而全绿。
  trow.getBoundingClientRect = () => ({ x: 16, y: 535, width: 320, height: 60 });
  tools.getBoundingClientRect = () => ({ x: 24, y: 535, width: 92, height: 44 });
  trailing.getBoundingClientRect = () => ({ x: 186, y: 535, width: 142, height: 44 });

  const dock = mk('div', 'dock');
  const pillRow = mk('div', 'pills');
  const pillA = mk('span');
  const statA = mk('button');
  // ⚠️ **文字必须真的写进去** —— 只设 aria-label 是不够的：
  //    真机上这两个按钮**既有** aria-label **也有**可见文字。
  //    只给 aria-label 的 fixture 会让 findStatButtons 一条都找不到，
  //    断言就变成「测一个我编出来的结构」。
  statA.setAttribute('aria-label', '11 turns 446 steps · 103 tok/s');
  statA.textContent = '11 turns 446 steps·103 tok/s';
  pillA.append(statA);
  const pillB = mk('span');
  const statC = mk('button');
  statC.setAttribute('aria-label', '89.2M tok · Cache hit 98%');
  statC.textContent = '89.2M tok·Cache hit 98%';
  pillB.append(statC);
  const ring = mk('span');
  const statB = mk('button');
  statB.setAttribute('aria-label', '79% of context used');
  statB.textContent = '79%';
  ring.append(statB);
  pillRow.append(pillA, pillB);
  dock.append(pillRow, ring);
  compRoot.append(card, dock);
  cbar.append(compRoot);
  body.append(cbar);

  tick(); tick();

  // ① 统计区被打上标记，而且**打在该打的那个上**（dock），不是 composer 根
  const marked = body.querySelectorAll('[data-qx-status]');
  if (!marked.length) {
    console.log('FAIL 统计区没有被打上 data-qx-status（状态条不会被收起）');
    process.exit(1);
  }
  if (marked.indexOf(dock) === -1) {
    console.log('FAIL 标记没落在统计区上，落在：'
      + marked.map((e) => e.className).join(',') + ' —— 统计条会留在屏幕上');
    process.exit(1);
  }
  if (marked.some((e) => e === compRoot || e === card)) {
    console.log('FAIL 标记误落在 composer 根/输入卡片上 —— B12 会把整个输入框藏起来');
    process.exit(1);
  }
  console.log('PASS 统计区被精确标记（不含输入卡片与 composer 根）');

  // ② 状态按钮插在 tools 与 trailing **之间**，且不在行尾
  const sbtn = body.querySelector('.qx-status-btn');
  if (!sbtn) {
    console.log('FAIL 工具栏里没有状态按钮');
    process.exit(1);
  }
  if (sbtn.parentNode !== trow) {
    console.log('FAIL 状态按钮不在工具栏行里（parent='
      + (sbtn.parentNode && sbtn.parentNode.className) + '）');
    process.exit(1);
  }
  const order = trow.children.map((c) => c.className || c.getAttribute('data-slot') || '');
  const iTools = order.indexOf('tools');
  const iSend = order.indexOf('trailing');
  if (!(iTools >= 0 && iSend > iTools)) {
    console.log('FAIL 工具栏结构不对：' + JSON.stringify(order));
    process.exit(1);
  }
  if (!(iTools < order.indexOf('qx-status-btn') && order.indexOf('qx-status-btn') < iSend)) {
    console.log('FAIL 状态按钮没排在 tools 与 trailing 之间，实得：' + JSON.stringify(order)
      + ' —— 排在发送键右边会把读数放到主操作旁边');
    process.exit(1);
  }
  console.log('PASS 状态按钮插在 tools 与 trailing 之间（发送键左边）');

  // ③ 按钮上显示的是**上下文占用**，不是缓存命中率
  const val = sbtn.querySelector('.qx-status-val');
  const shown = val ? (val.textContent || '').trim() : '';
  if (shown !== '79%') {
    console.log('FAIL 按钮读数是「' + shown + '」，应为 79%（上下文占用）'
      + ' —— 宽泛匹配 /\d+%/ 会先撞上「Cache hit 98%」');
    process.exit(1);
  }
  console.log('PASS 按钮读数取的是上下文占用（79%），不是缓存命中率（98%）');

  // ④ 点按钮 → 弹层里**三条统计一个不少**
  sbtn.click();
  const sheet = body.querySelector('[data-qx-statussheet]');
  if (!sheet) {
    console.log('FAIL 点状态按钮没有弹出统计层');
    process.exit(1);
  }
  const rows = sheet.querySelectorAll('.qx-stat-row').map((r) => (r.textContent || '').trim());
  if (rows.length !== 3) {
    console.log('FAIL 统计层只有 ' + rows.length + ' 行，实得 ' + JSON.stringify(rows));
    process.exit(1);
  }
  const joined = rows.join(' | ');
  if (joined.indexOf('turns') === -1 || joined.indexOf('Cache') === -1
      || joined.indexOf('context') === -1) {
    console.log('FAIL 统计层字段不全（收进按钮不等于可以少字段）：' + joined);
    process.exit(1);
  }
  console.log('PASS 点按钮弹出统计层，三个字段一个不少');

  // ⑤ **自己造的按钮不能被当成 DSH 的统计按钮**
  //    这条曾经把整个 composer 藏掉：药丸文字是「79%」，
  //    同样命中统计判据 → findStatRow 一路走到 composer 根。
  if (body.querySelectorAll('[data-qx-status]').some((e) => e.contains(sbtn))) {
    console.log('FAIL 自己造的状态按钮被算进了统计区');
    process.exit(1);
  }
  sheet.remove();

  // ── 提示条不能重复（必须放在最后：它会 dispose 掉当前这一轮）──────
  // ⚠️ 真机症状：定制层热替换后，DSH 行里出现**两个**「上下文 79%」。
  //   原因：提示条插在 DSH 自己的行里（不在我们的 layer 里），
  //   dispose 掉 layer 带不走它们；新 mount 的 state.painted 又是空的。
  // 这里真跑一次「dispose → 再 mount」，复现整条热替换路径。
  // ⚠️ 数的对象换成**所有提示条**（`.qx-hint-` 前缀）：
  //    上下文标签已按用户要求从列表移除，`.qx-hint-sparse` 是唯一还在插的。
  //    断言的对象必须跟着产品行为走，否则它会默默测不到任何东西。
  const countHint = (r) => r.children.filter(
    (c) => c.className.indexOf('qx-hint-') === 0).length;
  const before = treeRows.map(countHint);
  // 注意：不是每行都该有提示 —— AAA 是稀疏行（本就没有），
  // ZZZ 不在列表里、Ws 是工作区行。所以只查「没超过 1」。
  if (before.some((n) => n > 1)) {
    console.log('FAIL 第一轮就有重复：' + JSON.stringify(before));
    process.exit(1);
  }
  if (before.filter((n) => n === 1).length !== 1) {
    console.log('FAIL 第一轮该有一条稀疏提示（AAA），实得 ' + JSON.stringify(before));
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
  const after = treeRows.map(countHint);
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

  // ── 第 9 版：消息操作按钮补到 44×44 ─────────────────────────────────
  // 🔴 这条断言背后是一个**我自己造成的**回归：
  //   markShortTapRows 里写着 setProperty('min-width','32px','important')，
  //   理由是「间距只有 40px，补到 44 会重叠」——
  //   而实测那一行只占 200px、视口 359，右边空着 133px，**根本不会重叠**。
  //   我拿「行内间距」当成了「可用宽度」，压了整整一轮。
  //
  //   断言盯两件事：
  //   1) 32×40 的按钮被顶到 44×44（不能停在 32）
  //   2) 已经够大的按钮（188×44）**一个像素都不许动** ——
  //      写成 width:44px 会把它压扁，这条能抓住那种写法。
  const node = makeEl('div');
  node.setAttribute('data-slot', 'conversation.chat.node');
  const col = makeEl('div');
  col.className = 'EvIC1a_column';
  node.appendChild(col);
  // ⚠️ 必须挂一层 `conversation.session` 祖先 ——
  //    markShortTapRows 是 `querySelector('[data-slot="conversation.session"]')`
  //    找不到就**整段 return**。少这一层时断言报 minW=undefined，
  //    那是**夹具不对**，不是代码不对 —— 两者必须分清，
  //    否则就会去「修」一个根本没坏的函数。
  const sess9 = makeEl('div');
  sess9.setAttribute('data-slot', 'conversation.session');
  sess9.appendChild(node);
  body.appendChild(sess9);

  // ⚠️ 夹具故意从 **32** 起步，而不是原始的 24–27：
  //    32 是**卡死状态** —— 旧版把它们设成 32 之后，
  //    `if (r.width >= 32) continue` 让它们再也升不上去。
  //    只用 27 起步的话这条缺陷测不出来（27 < 32，会被处理）。
  const small = makeEl('button');
  small.className = 'xzv4MW_action';
  small.setAttribute('aria-label', 'Copy');
  small.getBoundingClientRect = () => ({ x: 26, y: 525, width: 32, height: 40 });
  const wide = makeEl('button');
  wide.className = 'xzv4MW_trigger';
  wide.setAttribute('aria-label', 'Completed in 5m 37s');
  wide.getBoundingClientRect = () => ({ x: 234, y: 520, width: 188, height: 44 });
  col.appendChild(small);
  col.appendChild(wide);

  tick();
  // ⚠️ 断言看的是**行内值真的落了地**，不是「我们调用过 setProperty」。
  //    第 8 版那处 32px 就是调用成功、值也写对了，但值本身就是错的。
  // ⚠️ 还要看 **important**：这是本轮查明的关键机制 ——
  //    这些按钮的 min-width 只能靠行内 !important 改得动。
  //    不带 important 的正确写法在真机上依然无效，却能骗过所有断言。
  const w9 = small.style.map['min-width'];
  const h9 = small.style.map['min-height'];
  if (w9 !== '44px' || h9 !== '44px') {
    console.log('FAIL 消息操作按钮没补到 44：minW=' + w9
      + ' minH=' + h9 + '（上一版错误地停在 32×40）');
    process.exit(1);
  }
  if (small.style.imp['min-width'] !== 'important'
      || small.style.imp['min-height'] !== 'important') {
    console.log('FAIL 行内 min-* 漏了 !important —— 真机上改不动（'
      + JSON.stringify(small.style.imp) + '），闸门必须挡住这种"看着对"');
    process.exit(1);
  }
  if (wide.style.map['min-width']) {
    console.log('FAIL 已经够大的按钮被改了 min-width=' + wide.style.map['min-width']
      + '（应该只补小的，别把 188×44 的压扁）');
    process.exit(1);
  }
  console.log('PASS 消息操作按钮补到 44×44，且不碰本来就够大的按钮');

  // ── 第 10 版：Trajectory 页的两类过小控件 ──────────────────────────
  //
  // 这一版第一次去看 Trajectory 页，立刻撞上两处从来没发现过的问题：
  //
  //  (A) 左侧「请求标记」16×16 × 14 个 ——
  //      它是「跳到第 N 次请求」的唯一入口，16px 在手机上永远点不准。
  //      圆点是 ::before 伪元素（5×5），按钮自身透明无子元素，
  //      所以**放大热区不会动那个点**。
  //  (B) 顶栏 "Collapse turns" 55×20 / "Collapse calls" 51×20 ——
  //      父容器本来就 44 高，是按钮自己矮了一半。
  //
  // (A) 断言的是**三件事一起**，少一件都不算修好：
  //   1) 补到 44×44
  //   2) **负边距存在且对称** —— 不然行高从 30 撑到 44，
  //      14 行就是 +196px 滚动高度，滚动条明显变短
  //   3) **带 !important** —— 第 9 轮查明：没有它，真机上根本改不动，
  //      而值写对了，能骗过只看值的断言
  //
  // ⚠️ 夹具必须同时具备：透明背景 + 无子元素。
  //    判据少一个就会**误伤** —— 有背景色的小方块可能是色块本身，
  //    有子元素的是真按钮。这不是夹具凑数，是判据本身的一部分。
  const dot = makeEl('button');
  dot.setAttribute('aria-label', 'Request #1');
  dot.getBoundingClientRect = () => ({ x: 6, y: -418, width: 16, height: 16 });
  sess9.appendChild(dot);

  // 反例一：有背景色的 12×12 色块 —— **不该**被拉大
  const solid = makeEl('button');
  solid.className = 'Swatch_color';
  solid.__cs = { backgroundColor: 'rgb(173, 178, 184)' };   // 不透明色块
  solid.getBoundingClientRect = () => ({ x: 100, y: 200, width: 12, height: 12 });
  sess9.appendChild(solid);
  // 反例二：12×12 但**有子元素**的真按钮 —— 也不该被动
  const hasKid = makeEl('button');
  hasKid.getBoundingClientRect = () => ({ x: 200, y: 200, width: 12, height: 12 });
  hasKid.appendChild(makeEl('svg'));
  sess9.appendChild(hasKid);

  // (B) 工具条：父容器 185×44（flex），按钮 55×20
  const trajBar = makeEl('div');
  trajBar.getBoundingClientRect = () => ({ x: 6, y: 96, width: 185, height: 44 });
  const collapse = makeEl('button');
  collapse.setAttribute('aria-label', 'Collapse turns');
  collapse.getBoundingClientRect = () => ({ x: 83, y: 108, width: 55, height: 20 });
  trajBar.appendChild(collapse);
  sess9.appendChild(trajBar);
  // 反例三：父容器是段落（高 300）—— 绝对不能被撑成 44 一行
  const paraBox = makeEl('div');
  paraBox.getBoundingClientRect = () => ({ x: 0, y: 400, width: 320, height: 300 });
  const btnInPara = makeEl('button');
  btnInPara.getBoundingClientRect = () => ({ x: 20, y: 420, width: 60, height: 18 });
  paraBox.appendChild(btnInPara);
  sess9.appendChild(paraBox);

  tick();
  if (dot.style.map['min-width'] !== '44px' || dot.style.map['min-height'] !== '44px') {
    console.log('FAIL 请求标记没补到 44：'
      + JSON.stringify({ w: dot.style.map['min-width'], h: dot.style.map['min-height'] })
      + '（16×16 在手机上永远点不准，而它是唯一的跳转入口）');
    process.exit(1);
  }
  if (dot.style.imp['min-width'] !== 'important') {
    console.log('FAIL 请求标记漏了 !important —— 真机上改不动（' + JSON.stringify(dot.style.imp) + '）');
    process.exit(1);
  }
  const ml = parseFloat(dot.style.map['margin-left']);
  const mr = parseFloat(dot.style.map['margin-right']);
  const mt = parseFloat(dot.style.map['margin-top']);
  // ⚠️ 对称 = 两值**相等**（都是 −14），不是「相加为零」。
  //    写成 Math.abs(ml + mr) 会把完全正确的 −14/−14 判成不对称 ——
  //    这是我自己在写断言时犯的错，症状同样是「看起来像产品代码坏了」。
  if (!(ml < 0) || !(mr < 0) || Math.abs(ml - mr) > 0.01) {
    console.log('FAIL 请求标记的负边距不对称：margin-left=' + dot.style.map['margin-left']
      + ' margin-right=' + dot.style.map['margin-right']
      + ' —— 不对称会把圆点挤偏（布局盒子变成 44−14 而不是 16）');
    process.exit(1);
  }
  if (!(mt < 0)) {
    console.log('FAIL 请求标记缺 margin-top 负边距，行高会从 30 撑到 44');
    process.exit(1);
  }
  if (solid.style.map['min-width'] || hasKid.style.map['min-width']) {
    console.log('FAIL 误伤了正常控件：有背景色的色块/有子元素的按钮被拉大了');
    process.exit(1);
  }
  if (collapse.style.map['min-height'] !== '44px') {
    console.log('FAIL 工具条矮按钮没补到 44：min-height=' + collapse.style.map['min-height']
      + '（父容器本来就 44 高，补齐不改变任何布局）');
    process.exit(1);
  }
  if (btnInPara.style.map['min-height']) {
    console.log('FAIL 误伤了段落里的按钮：min-height=' + btnInPara.style.map['min-height']
      + '（父容器 300 高，补到 44 会把段落撑开一行）');
    process.exit(1);
  }
  console.log('PASS 请求标记 16→44（负边距对称+important），工具条矮按钮补到 44，不误伤色块/真按钮/段落');

  // ── 第 11 轮新增：「有」不等于「有且仅有一个」──────────────────────
  //
  // 放在**最后**是有原因的：这条会真的再跑一次 mount，
  // 而 mount 会重新装一遍监听器。如果插在中间，
  // 后面依赖「监听器装了几个」的断言会被它污染（实测踩过：
  // 「没装上『选完自动收起』的监听」当场变红，而那条其实没问题）。
  //
  // 实测热重载多次后页面上同时存在 **2 个** .qx-layer，
  // 而原来的断言照样 PASS —— 因为它只问「在不在」，不问「有几个」。
  // 它们 pointer-events:none、没有子元素，谁也挡不住：
  // **肉眼看不出、截图拍不出、功能测试全绿，只有数量会变。**
  //
  // ⚠️ 必须**真跑一次 mount**（而且是**不 dispose 就再跑**，模拟热重载竞态）。
  //    如果断言里自己写一遍清理逻辑、再检查自己清得对不对，
  //    那测的是闸门自己 —— 删掉 custom.js 那行它照样绿。
  //    第 9 轮记过：闸门必须测**产品代码**，不是测替身。
  const nBase = body.querySelectorAll('.qx-layer').length;
  const staleLayer = makeEl('div');
  staleLayer.className = 'qx-layer';
  body.appendChild(staleLayer);            // 冒充上一次 mount 的遗留物
  // ⚠️ 不要假设「此刻页面上恰好有 1 个层」—— 前面那条 dispose 幂等测试
  //    刚把层删光，基线是 0。写死期望值会得到一条
  //    「夹具问题」的假失败，症状和真失败一模一样。
  //    这里只断言**确实多了一个**，真要求留到最后：mount 之后必须是 1。
  const n0 = body.querySelectorAll('.qx-layer').length;
  if (n0 !== nBase + 1) {
    console.log('FAIL 夹具问题：遗留层没挂上（' + nBase + ' → ' + n0 + '）');
    process.exit(1);
  }
  let dispose3 = null;
  try {
    dispose3 = registered({ document, window, root: makeEl('div') });
    if (dispose3 && typeof dispose3.then === 'function') dispose3 = await dispose3;
  } catch (e) {
    console.log('FAIL 第二次 mount 抛错: ' + e.message);
    process.exit(1);
  }
  const n1 = body.querySelectorAll('.qx-layer').length;
  if (n1 !== 1) {
    console.log('FAIL 二次 mount 后页面上有 ' + n1 + ' 个 qx-layer（应为 1）'
      + ' —— 热重载会一直累积。它们 pointer-events:none、无子元素，'
      + '所以肉眼/截图/功能测试全都发现不了，只有这个数会变');
    process.exit(1);
  }
  if (typeof dispose3 === 'function') dispose3();
  console.log('PASS 二次 mount 会清掉上一次遗留的 qx-layer（只剩一个，不累积）');

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
