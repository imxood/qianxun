/*
 * 千寻移动端 · L3 定制脚本（custom.js） —— 第 2 版：导航重建
 * =============================================================================
 * 部署到：<dsh_home>/mobile-access/custom.js
 *
 * ============================== 入口纪律（最重要的一条）==============================
 *
 * ⚠️⚠️ **入口必须是函数表达式，不能是立即执行的 async 箭头。**
 *
 * 踩过的坑（语法合法、服务正常、hash 正确，界面却**完全不动**）：
 *
 *     window.qxMobile.register((async (ctx) => { ... })())
 *
 * 那写法立即执行并返回 **Promise**。bootstrap.js:48 拿到 mount 的返回值
 * 当清理函数：
 *
 *     const result = mount({ root: ensureRoot(), document, window })
 *     dispose = typeof result === 'function' ? result : null
 *
 * Promise 不是 function -> dispose 永远是 null；而 mount 内部的逻辑
 * 要等网络返回才跑 —— 于是按钮永远不出现。
 *
 * 恶劣之处：**没有任何报错**。bootstrap 只在 catch 里打 console，
 * 而它 catch 的是同步异常；一个「类型不对但没抛」的返回值一路静默通过。
 * 唯一线索是「界面没动」，极易误判成 CSS 没生效。
 *
 * 正确：register(async function qxMount(ctx) { ... })
 *
 * ============================== 另外两条不能违反的律 ==============================
 *
 * ① 选择器只能用 data-* / role / aria-*。
 *    DSH 的 class 是**构建期哈希**（实测 hHd-Xa_panelRow / bhn1Oq_searchButton /
 *    YDXeBa_projectRow），每次 DSH 构建都变。清单见 docs/probe/dom-anchors.json
 *    （从 677KB bundle 实测）。用了 class 就会静默失效。
 *
 * ② 不隐藏 DSH 原版界面。
 *    想过 `.dshm-shell { display:none }` 一刀切 —— 那是「重写」不是「重排」，
 *    会把 141 个 RPC 的 UI 工作量全揽过来。正确顺序是逐块替换。
 *    本版只**加**一个覆盖层，DSH 自己的界面在下面照常工作。
 *
 * ============================== 第 2 版做了什么 ==============================
 *   · 全屏覆盖层（☰ 按钮 / 左边缘右滑）
 *   · 侧栏 = 工作区树 + 会话列表（实测 8 个工作区 · 394 条）
 *   · 会话行按实测认知渲染：稀疏行**不补**「0 轮 0 步 上下文 0%」
 *   · 待答分三档：pending / none / unknown，绝不把 unknown 说成「没有」
 *   · 工具卡加「看全文」入口（12KB 阈值 + 底部 sheet）
 *
 * ============================== 线格式（全部实测）==============================
 *   POST /api/<ns>/<method>
 *   method: **全名** `session/list`（写裸 `list` 会得到
 *           method "list" does not match endpoint "session/list"）
 *   payload.args: **内层还有一层 request**
 *     { args: { request: { …实际参数… } } }
 *     唯一例外 session/list：平铺，键名恰好叫 `_request`
 *   响应: { type:'server-response', rpcId, result:{ ok, value|error } }
 *   throughSeq 是**游标**不是任意上限（取 projections.asOfSeq）
 */

window.qxMobile.register(async function qxMount({ document: doc, window: win }) {

  // ══════════════════════════════════════════════════════════════════
  // 常量
  // ══════════════════════════════════════════════════════════════════

  /** 折叠工具行的字节阈值。略高于实测 p90（9.4KB）。 */
  const FOLD_BYTES = 12 * 1024
  /** 每个工作区默认显示几条。实测最多 192 条，全列出来没法用。 */
  const ROWS_PER_WS = 8

  // ══════════════════════════════════════════════════════════════════
  // 状态
  // ══════════════════════════════════════════════════════════════════

  const state = {
    sessions: new Map(),      // sessionId → summary
    byWorkspace: new Map(),   // cwd → summary[]
    listSyncedAt: null,
    lastError: null,
    drawerOpen: false,
    /** 展开的工作区集合。空集合 = 全部按默认条数。 */
    expanded: new Set(),
    currentSessionId: null,
  }

  let rpcSeq = 0

  // ══════════════════════════════════════════════════════════════════
  // RPC
  // ══════════════════════════════════════════════════════════════════

  async function rpc(namespace, method, args, argsKey) {
    const key = argsKey || 'request'
    const res = await win.fetch('/api/' + namespace + '/' + method, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        type: 'client-request',
        rpcId: 'qx-m' + (++rpcSeq),
        method: namespace + '/' + method,
        payload: { args: { [key]: args } },
      }),
    })
    if (!res.ok) throw new Error('HTTP ' + res.status + ' ' + res.statusText)
    const body = await res.json()
    const result = body && body.result
    if (!result) throw new Error('响应里没有 result 字段')
    if (result.ok !== true) {
      const err = result.error || {}
      throw new Error((err.code || 'unknown') + ': ' + (err.message || JSON.stringify(err)))
    }
    return result.value
  }

  /** ⚠️ session/list 是**唯一**平铺的方法，参数键恰好叫 `_request`。 */
  const listSessions = () => rpc('session', 'list', {}, '_request')

  function pageSession(sessionId, asOfSeq, maxMessages) {
    return rpc('session', 'page', {
      address: { kind: 'session', sessionId },
      throughSeq: asOfSeq,
      maxMessages: maxMessages || 50,
    })
  }

  // ══════════════════════════════════════════════════════════════════
  // 摘要解析 —— 稀疏行认知的落点
  // ══════════════════════════════════════════════════════════════════

  /**
   * 把一条 `session/list` item 解析成列表行要用的摘要。
   *
   * ⚠️ **缺字段 ≠ 零。** 实测 394 条：
   *   只有 title 的会话 **208 条 (52.8%)** —— 键**根本不存在**，
   *   不是 null，不是 {x:0}。那是「从未跑过一轮」。
   *   按固定结构解析并兜底 0，会给一半以上的会话画出
   *   假的「上下文 0%」和假的「已完成」。
   */
  function readProjection(item) {
    const values = (item && item.projections && item.projections.values) || {}
    const has = (key) => Object.prototype.hasOwnProperty.call(values, key)

    // 只有 title 的会话 = 从未跑过一轮
    const sparse = !has('sessionStats') && !has('turnOutline')

    const stats = has('sessionStats') ? values.sessionStats : null
    const outline = has('turnOutline') ? (values.turnOutline || []) : null
    const pressure = has('contextPressure') ? values.contextPressure : null
    const timing = has('subagentTiming') ? values.subagentTiming : null
    const team = has('agentTeam') ? values.agentTeam : null
    const questions = has('userQuestions') ? values.userQuestions : null

    // 百分比只能用 projectedTokens / contextWindow（只有它给分母）
    const contextPct = pressure && pressure.contextWindow
      ? (pressure.projectedTokens / pressure.contextWindow) * 100
      : null

    // 待答**三档** —— 真机实测（394 条）：
    //   有 userQuestions 键    84 条 (21.3%) → pending / none
    //   无键 且稀疏            208 条 (52.8%) → **none**（从未跑过，不可能有）
    //   无键 但**有**轮次历史  102 条 (25.9%) → **unknown**（真不可知）
    //
    // 前两档界面**同形**。只有第三档不能说「没有待答」
    // （否定断言律：没收到 ≠ 不存在）。
    const pendingKind = questions
      ? ((questions.active || []).length > 0 ? 'pending' : 'none')
      : (sparse ? 'none' : 'unknown')

    return {
      sessionId: item.sessionId,
      title: values.title || '（无标题）',
      cwd: item.cwd || '',
      running: item.running === true,
      updatedAt: item.updatedAt || 0,
      sparse,
      pendingKind,
      turns: outline ? outline.length : null,
      steps: stats && typeof stats.steps === 'number' ? stats.steps : null,
      contextPct,
      lastTurnCompleted: timing ? timing.lastTurnCompleted === true : null,
      teamSize: team && team.members ? team.members.length : null,
      lastResponse: outline && outline.length > 0
        ? (outline[outline.length - 1].response || '')
        : null,
    }
  }

  function groupByWorkspace(list) {
    const map = new Map()
    for (const item of list) {
      const key = item.cwd || '__ungrouped__'
      if (!map.has(key)) map.set(key, [])
      map.get(key).push(item)
    }
    for (const rows of map.values()) rows.sort((a, b) => b.updatedAt - a.updatedAt)
    return map
  }

  async function refreshSessions() {
    const value = await listSessions()
    const items = (value && value.items) || []
    const list = []
    state.sessions.clear()
    for (const item of items) {
      const parsed = readProjection(item)
      state.sessions.set(parsed.sessionId, parsed)
      list.push(parsed)
    }
    state.byWorkspace = groupByWorkspace(list)
    state.listSyncedAt = Date.now()
    state.lastError = null
    return list
  }

  // ══════════════════════════════════════════════════════════════════
  // DOM 小工具
  // ══════════════════════════════════════════════════════════════════

  const el = (tag, props, children) => {
    const node = doc.createElement(tag)
    if (props) {
      for (const key of Object.keys(props)) {
        const value = props[key]
        if (value === null || value === undefined || value === false) continue
        if (key === 'class') node.className = value
        else if (key === 'text') node.textContent = value
        else if (key === 'style') node.setAttribute('style', value)
        else if (key.slice(0, 2) === 'on' && typeof value === 'function') {
          node.addEventListener(key.slice(2).toLowerCase(), value)
        } else {
          node.setAttribute(key, value === true ? '' : String(value))
        }
      }
    }
    const kids = Array.isArray(children) ? children : [children]
    for (const child of kids) {
      if (child === null || child === undefined || child === false) continue
      node.appendChild(typeof child === 'string' ? doc.createTextNode(child) : child)
    }
    return node
  }

  const fmtAgo = (ms) => {
    if (!ms) return ''
    const s = Math.floor((Date.now() - ms) / 1000)
    if (s < 60) return '刚刚'
    if (s < 3600) return Math.floor(s / 60) + ' 分钟前'
    if (s < 86400) return Math.floor(s / 3600) + ' 小时前'
    const d = Math.floor(s / 86400)
    if (d < 30) return d + ' 天前'
    const mo = Math.floor(d / 30)
    return mo < 12 ? mo + ' 个月前' : Math.floor(mo / 12) + ' 年前'
  }

  const wsName = (cwd) => {
    if (!cwd) return '（未归属）'
    const parts = cwd.split(/[\\/]/).filter(Boolean)
    return parts[parts.length - 1] || cwd
  }

  const ellipsis = (text, max) => {
    const s = String(text === null || text === undefined ? '' : text)
    return s.length <= max ? s : s.slice(0, max - 1) + '…'
  }

  const note = (text) => {
    if (win.console) win.console.log('[qx-mobile]', text)
  }

  // ══════════════════════════════════════════════════════════════════
  // 会话行
  // ══════════════════════════════════════════════════════════════════

  /**
   * 一行会话。
   *
   * ⚠️ **稀疏行是 52.8%，不是边角情况**，所以它是一个**认真设计过的形态**：
   *   满行   标题 / agent 最近做了什么 / 状态+轮次+步数+上下文
   *   稀疏行 标题 / 「还没有发过消息」 / 状态（只这一个）
   *
   * 绝**不**给稀疏行补「0 轮 · 0 步 · 上下文 0%」——
   * 那是把「从未发生」说成「发生了但结果为零」。
   */
  function sessionRow(summary, onOpen) {
    const tone = summary.pendingKind === 'pending' ? 'wait'
      : summary.running ? 'run'
      : summary.lastTurnCompleted === true ? 'done'
      : summary.lastTurnCompleted === false ? 'weak'
      : 'sparse'

    const word = summary.pendingKind === 'pending' ? '待审批'
      : summary.running ? '运行中'
      : summary.sparse ? '空闲'
      : summary.lastTurnCompleted === true ? '已完成'
      : summary.lastTurnCompleted === false ? '已收工'
      : '状态不明'

    const second = summary.sparse
      ? el('div', { class: 'qx-s-act qx-s-sparse', text: '还没有发过消息' })
      : el('div', {
          class: 'qx-s-act',
          text: summary.lastResponse
            ? ellipsis(summary.lastResponse.replace(/\s+/g, ' ').trim(), 44)
            : (summary.turns || 0) + ' 轮 · 还没有回复',
        })

    // 只有真的有值才显示。缺字段**整块不显示**，不是显示 0。
    const meta = [el('span', { class: 'qx-s-st qx-s-st--' + tone, text: word })]
    if (summary.turns !== null) meta.push(el('span', { text: summary.turns + ' 轮' }))
    if (summary.steps !== null) meta.push(el('span', { text: summary.steps + ' 步' }))
    if (summary.teamSize) {
      meta.push(el('span', { class: 'qx-s-chip', text: '子 ' + summary.teamSize }))
    }
    // 上下文 >50% 才显示（实测 p90=53.2%，常驻等于给 87% 的会话腾地方）
    if (summary.contextPct !== null && summary.contextPct > 50) {
      meta.push(el('span', {
        class: 'qx-s-ctx' + (summary.contextPct > 65 ? ' qx-s-ctx--hi' : ''),
        text: '上下文 ' + Math.round(summary.contextPct) + '%',
      }))
    }

    return el('button', {
      class: 'qx-s-row' + (summary.pendingKind === 'pending' ? ' qx-s-row--pending' : ''),
      type: 'button',
      'data-qx-session': summary.sessionId,
      onclick: () => onOpen(summary),
    }, [
      el('div', { class: 'qx-s-rail qx-s-rail--' + tone }),
      el('div', { class: 'qx-s-main' }, [
        el('div', { class: 'qx-s-head' }, [
          el('div', { class: 'qx-s-title', text: summary.title }),
          el('div', { class: 'qx-s-age', text: fmtAgo(summary.updatedAt) }),
        ]),
        second,
        el('div', { class: 'qx-s-meta' }, meta),
      ]),
    ])
  }

  // ══════════════════════════════════════════════════════════════════
  // 覆盖层
  // ══════════════════════════════════════════════════════════════════

  let layer = null
  let toggleNode = null
  let disposeSwipe = null
  let pollTimer = 0

  function ensureLayer() {
    if (layer && layer.isConnected) return layer
    layer = el('div', { class: 'qx-layer', 'data-qx-layer': '' })
    layer.style.zIndex = '2147483644'
    doc.body.appendChild(layer)
    return layer
  }

  function renderDrawer() {
    const groups = []
    state.byWorkspace.forEach((rows, cwd) => groups.push([cwd, rows]))
    groups.sort((a, b) => b[1].length - a[1].length)

    const list = el('div', { class: 'qx-d-list' })

    if (state.lastError) {
      list.append(el('div', {
        class: 'qx-d-empty',
        text: '拉取失败：' + state.lastError,
      }))
    } else if (groups.length === 0) {
      list.append(el('div', { class: 'qx-d-empty', text: '正在拉取会话…' }))
    } else {
      for (let g = 0; g < groups.length; g += 1) {
        const cwd = groups[g][0]
        const rows = groups[g][1]
        const shown = state.expanded.has(cwd) ? rows.length : Math.min(ROWS_PER_WS, rows.length)
        let sparse = 0
        let pending = 0
        for (const r of rows) {
          if (r.sparse) sparse += 1
          if (r.pendingKind === 'pending') pending += 1
        }

        list.append(el('button', {
          class: 'qx-d-ws', type: 'button', 'data-qx-ws': cwd,
          onclick: () => {
            if (state.expanded.has(cwd)) state.expanded.delete(cwd)
            else state.expanded.add(cwd)
            paint()
          },
        }, [
          el('span', { class: 'qx-d-ws-caret', text: state.expanded.has(cwd) ? '▾' : '▸' }),
          el('span', { class: 'qx-d-ws-name', text: wsName(cwd) }),
          el('span', { class: 'qx-d-ws-meta' }, [
            pending ? el('span', { class: 'qx-d-ws-pending', text: '待答 ' + pending }) : null,
            el('span', { text: rows.length + ' 条' }),
            // 「稀疏 N」要弱化 —— 它是常态（52.8%），不该像警告
            sparse ? el('span', { class: 'qx-d-ws-sparse', text: '· 稀疏 ' + sparse }) : null,
          ]),
        ]))

        for (let i = 0; i < shown; i += 1) {
          list.append(sessionRow(rows[i], openSession))
        }
        if (shown < rows.length) {
          list.append(el('div', {
            class: 'qx-d-more',
            text: '展开其余 ' + (rows.length - shown) + ' 条',
          }))
        }
      }
    }

    const foot = el('div', { class: 'qx-d-foot' })
    if (state.listSyncedAt) {
      let pending = 0
      let unknown = 0
      state.sessions.forEach((s) => {
        if (s.pendingKind === 'pending') pending += 1
        if (s.pendingKind === 'unknown') unknown += 1
      })
      foot.append(el('span', { text: state.sessions.size + ' 条会话' }))
      foot.append(el('span', { text: '· 待答 ' + pending }))
      // ⚠️ 「无法确认」必须**单列**，不能并进「没有待答」——
      // 那是「没收到 ≠ 不存在」（否定断言律）
      if (unknown > 0) {
        foot.append(el('span', { class: 'qx-d-unknown', text: '· 无法确认 ' + unknown }))
      }
    }

    return el('div', { class: 'qx-drawer', 'data-qx-drawer': '' }, [
      el('div', { class: 'qx-d-head' }, [
        el('span', { class: 'qx-d-title', text: '会话' }),
        el('button', {
          class: 'qx-d-x', type: 'button', text: '✕', 'aria-label': '关闭',
          onclick: closeDrawer,
        }),
      ]),
      el('div', { class: 'qx-d-tools' }, [
        el('button', {
          class: 'qx-d-btn', type: 'button', text: '刷新', onclick: refreshAndRepaint,
        }),
        el('button', {
          class: 'qx-d-btn', type: 'button', text: '收起', onclick: closeDrawer,
        }),
      ]),
      list,
      foot,
    ])
  }

  function ensureToggle() {
    if (toggleNode && toggleNode.isConnected) return toggleNode
    toggleNode = el('button', {
      class: 'qx-toggle', type: 'button',
      'data-qx-toggle': '', 'aria-label': '打开会话列表', text: '☰',
      onclick: toggleDrawer,
    })
    toggleNode.style.zIndex = '2147483645'
    return toggleNode
  }

  function toggleDrawer() {
    state.drawerOpen = !state.drawerOpen
    paint()
  }

  function closeDrawer() {
    state.drawerOpen = false
    paint()
  }

  /**
   * 打开一个会话。
   *
   * ⚠️ 这里**不重写转录**。转录是 141 个 RPC 的 UI 价值所在，
   *    重写它等于放弃方案 B 的全部好处。我们只改 URL 并派发 popstate，
   *    让 DSH 自己的会话选择器接手。
   */
  function openSession(summary) {
    state.currentSessionId = summary.sessionId
    closeDrawer()
    const url = '/?session=' + encodeURIComponent(summary.sessionId)
    win.history.pushState({ qxSession: summary.sessionId }, '', url)
    try {
      win.dispatchEvent(new win.PopStateEvent('popstate', {
        state: { qxSession: summary.sessionId },
      }))
    } catch (error) {
      // 老 WebView 没有 PopStateEvent 构造器，退回普通事件
      win.dispatchEvent(new win.Event('popstate'))
    }
    note('打开会话 ' + summary.title)
  }

  async function refreshAndRepaint() {
    try {
      await refreshSessions()
    } catch (error) {
      state.lastError = String(error)
    }
    paint()
  }

  function paint() {
    if (!layer) return
    layer.replaceChildren()

    if (state.drawerOpen) {
      layer.append(el('div', { class: 'qx-scrim', onclick: closeDrawer }))
      layer.append(renderDrawer())
    }

    const toggle = ensureToggle()
    // 收起时才显示 —— 抽屉打开时它会挡住内容
    toggle.style.display = state.drawerOpen ? 'none' : 'block'
    layer.append(toggle)

    paintToolCards()
  }

  // ══════════════════════════════════════════════════════════════════
  // 工具卡：只**加**入口，不重写 DSH 的组件
  // ══════════════════════════════════════════════════════════════════

  /**
   * 给 DSH 自己的工具行加「看全文」入口。
   *
   * ⚠️ **不重写工具卡。** 只在末尾追加一个按钮。
   *
   * 依据实测（docs/design/12-transcript-scale.md）：
   *   工具事件 p50 = 94 B（小）；消息 p50 = 6.9 KB（大）
   *   p90 = 9.4 KB  -> 阈值定 12 KB
   *   单条最大 139 KB -> 139KB 塞进主视图会挤掉一切，必须进 sheet
   *
   * 选择器用 `data-disclosure-row`（实测存在的稳定语义属性）。
   */
  function paintToolCards() {
    let rows
    try {
      rows = doc.querySelectorAll('[data-disclosure-row]')
    } catch (error) {
      return
    }
    for (let i = 0; i < rows.length; i += 1) {
      const row = rows[i]
      if (row.getAttribute('data-qx-tool') === '1') continue
      row.setAttribute('data-qx-tool', '1')
      attachFullButton(row)
      // 展开态补挂（折叠时没内容可看）
      if (row.getAttribute('data-expanded') === 'true') {
        win.setTimeout(() => attachFullButton(row), 60)
      }
    }
  }

  function attachFullButton(row) {
    if (row.querySelector('[data-qx-full]')) return
    const text = (row.textContent || '').trim()
    if (!text || text.length < 400) return
    row.appendChild(el('button', {
      class: 'qx-full-btn', type: 'button',
      'data-qx-full': '',
      text: '看全文（约 ' + Math.round(text.length / 1024) + 'K）',
      onclick: (event) => {
        event.stopPropagation()
        openFullText(text)
      },
    }))
  }

  /** 底部 sheet 看全文。139KB 文本**惰性**进 sheet，不预渲染到主视图。 */
  function openFullText(text) {
    const sheet = el('div', { class: 'qx-sheet', 'data-qx-sheet': '' }, [
      el('div', { class: 'qx-sheet-grab' }),
      el('div', { class: 'qx-sheet-head' }, [
        el('span', { class: 'qx-sheet-title', text: '完整输出' }),
        el('span', { class: 'qx-sheet-size', text: Math.round(text.length / 1024) + 'K' }),
        el('button', {
          class: 'qx-sheet-x', type: 'button', text: '✕', 'aria-label': '关闭',
          onclick: () => sheet.remove(),
        }),
      ]),
      el('pre', { class: 'qx-sheet-body', text }),
      el('button', {
        class: 'qx-sheet-copy', type: 'button', text: '复制',
        onclick: () => {
          if (navigator.clipboard) navigator.clipboard.writeText(text)
          note('已复制')
        },
      }),
    ])
    doc.body.appendChild(sheet)
  }

  // ══════════════════════════════════════════════════════════════════
  // 左边缘右滑
  // ══════════════════════════════════════════════════════════════════

  function installEdgeSwipe() {
    let startX = 0
    let startY = 0
    let tracking = false

    const onStart = (event) => {
      const touch = event.touches && event.touches[0]
      if (!touch) return
      // 只从**左边缘 24px 内**起手。从屏幕中间起手会跟页面横滑打架。
      if (state.drawerOpen) return
      if (touch.clientX > 24) return
      startX = touch.clientX
      startY = touch.clientY
      tracking = true
    }
    const onMove = (event) => {
      if (!tracking) return
      const touch = event.touches && event.touches[0]
      if (!touch) return
      const dx = touch.clientX - startX
      const dy = touch.clientY - startY
      // 横向位移要明显大于纵向，否则判为纵向滚动（页面内的代码块横滑）
      if (dx > 60 && Math.abs(dy) < 40) {
        tracking = false
        toggleDrawer()
      }
    }
    const onEnd = () => { tracking = false }

    doc.addEventListener('touchstart', onStart, { passive: true })
    doc.addEventListener('touchmove', onMove, { passive: true })
    doc.addEventListener('touchend', onEnd, { passive: true })

    return () => {
      doc.removeEventListener('touchstart', onStart)
      doc.removeEventListener('touchmove', onMove)
      doc.removeEventListener('touchend', onEnd)
    }
  }

  // ══════════════════════════════════════════════════════════════════
  // 启动
  // ══════════════════════════════════════════════════════════════════

  // 先拉数据。失败也要把抽屉画出来 —— 用户需要看到「为什么空」
  try {
    await refreshSessions()
  } catch (error) {
    state.lastError = String(error)
  }

  ensureLayer()
  paint()

  disposeSwipe = installEdgeSwipe()

  // DSH 每次渲染转录都会换掉工具行 DOM，所以要重扫。
  // 页面可见时才扫，代价极低；漏扫的工具行（没有「看全文」入口）
  // 是用户看得见的缺陷。
  pollTimer = win.setInterval(() => {
    if (doc.visibilityState !== 'hidden') paintToolCards()
  }, 2000)

  note('L3 第2版：' + state.sessions.size + ' 条会话，'
    + state.byWorkspace.size + ' 个工作区，'
    + '错误=' + (state.lastError || '无'))

  // ⚠️ 必须返回**函数**（清理用）。bootstrap.js:48 判 typeof。
  return function qxDispose() {
    if (disposeSwipe) disposeSwipe()
    if (pollTimer) win.clearInterval(pollTimer)
    if (layer) { layer.remove(); layer = null }
    if (toggleNode) { toggleNode = null }
    for (const node of doc.querySelectorAll('[data-qx-sheet]')) node.remove()
  }
})
