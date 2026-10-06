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
function makeEl(tag) {
  const node = {
    tagName: String(tag).toUpperCase(),
    style: { setProperty(){}, removeProperty(){} },
    dataset: {},
    className: '',
    children: [],
    attrs: {},
    isConnected: true,
    hidden: false,
    textContent: '',
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
    addEventListener() {},
    removeEventListener() {},
    querySelector() { return null; },
    querySelectorAll() { return []; },
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
  addEventListener() {},
  removeEventListener() {},
});

let registered = null;
const window = {
  document,
  location: { pathname: '/', href: 'http://x/' },
  history: { pushState(){}, replaceState(){} },
  console,
  setTimeout: () => 0,
  clearTimeout() {},
  setInterval: () => 0,
  clearInterval() {},
  addEventListener() {},
  removeEventListener() {},
  dispatchEvent() {},
  fetch: () => Promise.resolve({
    ok: true,
    json: () => Promise.resolve({ result: { ok: true, value: { items: [] } } }),
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

  // ☰ 按钮必须真的进了 body。
  // ⚠️ 不能用 JSON.stringify(body) 找 —— 替身里 parentNode 与 children
  //    构成循环引用，stringify 直接抛 Converting circular structure。
  //    改成手写遍历，顺带把真实检查点打出来。
  let toggle = null;
  let scan = (node) => {
    for (const c of (node.children || [])) {
      if (c && c.className === 'qx-toggle') toggle = c;
      scan(c);
    }
  };
  scan(body);
  if (!toggle) {
    console.log('FAIL ☰ 按钮没有被插入 document.body');
    process.exit(1);
  }
  if (toggle.style.zIndex !== '2147483645') {
    console.log('FAIL ☰ 按钮 zIndex 不是 2147483645（实际 ' + toggle.style.zIndex + '）');
    process.exit(1);
  }
  console.log('PASS ☰ 按钮已插入且 zIndex 正确');
  dispose();
  console.log('PASS dispose 可调用');
  console.log('\n全部通过 ✓');
})();
"""


def main() -> int:
    if not TARGET.is_file():
        print(f"找不到 {TARGET}")
        return 1

    # 第一关：语法
    proc = subprocess.run(
        ["node", "--check", str(TARGET)], capture_output=True, text=True
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
        ["node", str(harness_path), str(TARGET)],
        capture_output=True, text=True, timeout=60,
    )
    print(proc.stdout.strip())
    if proc.stderr.strip():
        print(proc.stderr.strip()[:600])
    harness_path.unlink(missing_ok=True)
    return proc.returncode


if __name__ == "__main__":
    raise SystemExit(main())
