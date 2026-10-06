"""从 DSH 已渲染的 DOM 里读出「真实的工作区结构」—— 而不是用 cwd 猜。

为什么必须这样
--------------
方案 B 的 `custom.js` 跑在 DSH 页面里，页面**已经把 DSH 自己的工作区
树渲染好了**（6 个注册工作区 + 未分组区）。那份结构是权威的。

而我之前用 `session/list` 的 `cwd` 分组造了 8 个 —— 凭空多造 2 个，
还把 251 条「不属于任何工作区」的会话硬塞进了按目录猜的组。

⚠️ 但 bundle 里**搜不到** `ungrouped` / `workspaceIds` / `sessionIds`
（实测 633KB 主 bundle 全部为 0 次），说明工作区渲染在**按需 chunk** 里。
所以只能读 DOM，不能从 bundle 抠常量。

这个探针的职责：把 DSH 真实渲染出来的树结构 dump 出来，
作为 custom.js 改用「读 DOM 而非猜 cwd」的依据。

它跑在页面里（不是 Python 侧）。用法：
  1. 把本文件内容作为 custom.js 部署
  2. 手机进 DSH 页 → 点开 DSH 自己的「工作区」侧栏
  3. 结果打在 console，浮层显示
"""
(() => {
  'use strict'

  window.qxMobile.register(({ document, window }) => {

    const out = []
    const log = (label, value) => { out.push({ label, value }) }
    const text = (node) => (node ? (node.textContent || '').trim() : '')

    // ── 1. role=tree / treeitem 是实测存在的稳定语义锚 ──
    const trees = document.querySelectorAll('[role="tree"]')
    log('role=tree 数量', trees.length)
    for (let ti = 0; ti < trees.length; ti += 1) {
      const tree = trees[ti]
      const aria = tree.getAttribute('aria-label') || tree.getAttribute('aria-labelledby') || ''
      log(`tree[${ti}] aria-label`, aria || '(空)')

      const items = tree.querySelectorAll('[role="treeitem"]')
      log(`tree[${ti}] treeitem 数`, items.length)

      // 只取前两层，够看清结构了
      const sample = []
      for (let ii = 0; ii < items.length && ii < 24; ii += 1) {
        const item = items[ii]
        sample.push({
          level: item.getAttribute('aria-level') || '?',
          expanded: item.getAttribute('aria-expanded'),
          selected: item.getAttribute('aria-selected'),
          text: text(item).replace(/\s+/g, ' ').slice(0, 46),
          // 看看有没有我们能用的语义属性
          dataKeys: Object.keys(item.dataset || {}).sort().join(',') || '(无)',
          // 层级：算它前面有多少个同级 treeitem
          childTreeitems: item.querySelectorAll('[role="treeitem"]').length,
        })
      }
      log(`tree[${ti}] 前 ${sample.length} 项`, sample)
    }

    // ── 2. 找「未分组」那一区：它通常不是 treeitem ──
    // 扫描所有含「未分组 / Ungrouped / 展开其余」的节点
    const ungroupedHints = []
    const all = document.querySelectorAll('div, span, section')
    for (let i = 0; i < all.length && ungroupedHints.length < 8; i += 1) {
      const t = text(all[i])
      if (t && (t.includes('未分组') || t.includes('Ungrouped') || t.includes('展开其余'))) {
        if (t.length < 40) {
          ungroupedHints.push({
            text: t,
            dataKeys: Object.keys(all[i].dataset || {}).sort().join(',') || '(无)',
            tag: all[i].tagName,
            parentRole: all[i].parentElement
              ? (all[i].parentElement.getAttribute('role') || all[i].parentElement.tagName)
              : '',
          })
        }
      }
    }
    log('未分组/展开其余 提示节点', ungroupedHints)

    // ── 3. 全局可见的 data-* 统计（供 custom.js 选锚点）──
    const dataCount = {}
    const nodes = document.querySelectorAll('*')
    for (let i = 0; i < nodes.length; i += 1) {
      const keys = Object.keys(nodes[i].dataset || {})
      for (let k = 0; k < keys.length; k += 1) {
        const name = 'data-' + keys[k]
        dataCount[name] = (dataCount[name] || 0) + 1
      }
    }
    const sorted = Object.keys(dataCount)
      .sort((a, b) => dataCount[b] - dataCount[a])
      .slice(0, 30)
      .map((k) => k + '=' + dataCount[k])
    log('页面上的 data-* 统计（前 30）', sorted)

    // ── 4. role 统计 ──
    const roleCount = {}
    const roleNodes = document.querySelectorAll('[role]')
    for (let i = 0; i < roleNodes.length; i += 1) {
      const r = roleNodes[i].getAttribute('role')
      roleCount[r] = (roleCount[r] || 0) + 1
    }
    log('role 统计', Object.keys(roleCount)
      .sort((a, b) => roleCount[b] - roleCount[a])
      .map((k) => k + '=' + roleCount[k]))

    // ── 显示 ──
    const box = document.createElement('div')
    box.style.cssText = 'position:fixed;z-index:2147483647;left:8px;right:8px;'
      + 'top:calc(env(safe-area-inset-top) + 8px);max-height:80dvh;overflow:auto;'
      + 'background:#0b0e13;color:#e8edf2;border:1px solid rgba(255,255,255,.16);'
      + 'border-radius:12px;padding:12px 14px;font:11px/1.55 ui-monospace,Menlo,monospace;'
      + 'box-shadow:0 12px 40px rgba(0,0,0,.5)'

    const head = document.createElement('div')
    head.style.cssText = 'font-weight:700;font-size:13px;margin-bottom:8px;color:#FFC24B'
    head.textContent = 'DOM 结构探针（点开 DSH 自己的工作区侧栏后再看）'
    box.append(head)

    for (const entry of out) {
      const row = document.createElement('div')
      row.style.cssText = 'padding:4px 0;border-top:1px solid rgba(255,255,255,.07);word-break:break-all'
      const label = document.createElement('div')
      label.style.cssText = 'color:#12b5a8;font-weight:600'
      label.textContent = entry.label
      const value = document.createElement('div')
      value.style.cssText = 'color:#9AA7B4;white-space:pre-wrap'
      value.textContent = typeof entry.value === 'string'
        ? entry.value
        : JSON.stringify(entry.value, null, 1)
      row.append(label, value)
      box.append(row)
    }

    const close = document.createElement('button')
    close.textContent = '关闭'
    close.style.cssText = 'margin-top:10px;width:100%;padding:11px;border-radius:8px;'
      + 'border:1px solid rgba(255,255,255,.2);background:#1a2029;color:#e8edf2;font:600 13px system-ui'
    close.onclick = () => box.remove()
    box.append(close)

    document.body.append(box)
    if (window.console) window.console.log('[qx-dom-probe]', out)

    return () => box.remove()
  })
})()
