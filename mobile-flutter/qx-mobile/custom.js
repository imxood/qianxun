/*
 * 千寻移动端 · L3 定制脚本（custom.js） —— 第 1 版：通道自检 + 诊断浮层
 * =============================================================================
 * 部署到：<dsh_home>/mobile-access/custom.js
 * 入口：  window.qxMobile.register(({ root, document, window }) => …)
 *
 * ============================== 为什么第 1 版只做这个 ==============================
 * 这一版**不重建任何界面**。它做三件事：
 *   1. 验通道：window.fetch 能否带凭据调 session/list
 *   2. 验认知：字段签名的真实分布（52.8% 只有 title）
 *   3. 给后续版本一个可复用的 RPC 封装 + 状态存储
 *
 * 为什么不一次做完界面
 * --------------------
 * custom.js 是**热替换**的：改文件 5 秒后自动生效，语法错误只报 console
 * 不影响页面（bootstrap.js:95-100 失败保留旧版）。所以可以增量做。
 *
 * 但**不要**一上来就 `.dshm-shell { display:none }` 藏掉 DSH 界面
 * —— 那是「重写」不是「重排」，会把 141 个 RPC 的 UI 工作量全揽过来。
 * 正确顺序是 L1（CSS 窄屏，已在 custom.css）→ 逐块替换 → 最后才考虑全藏。
 *
 * ============================== 契约要点 ==============================
 * · 上下文只有 { root, document, window }，**没有 request**。
 *   脚本跑在同源页面里，window.fetch 本身就带 cookie，不用管 credentials。
 * · 一切选择器只能用 data-* / role / aria-*。
 *   ⚠️ class 名是构建期哈希（hHd-Xa_ / bhn1Oq_ / YDXeBa_），
 *      DSH 每次构建都变，用了必然静默失效。清单见 docs/probe/dom-anchors.json。
 * · 返回清理函数。热替换时 bootstrap 会先挂新 root 再卸旧 root。
 *
 * ============================== 线格式（实测）==============================
 * 端点  POST /api/<ns>/<method>
 * 信封  { type:'server-response', rpcId, result:{ ok, value | error } }
 * 参数  payload.args 是唯一合法键；⚠️ session/list 的参数恰好叫 _request
 * 取值  project 为 { type, rpcId, result } 三层，**不是** { value }
 */

(() => {
  'use strict'

  const state = {
    /** @type {Map<string, object>} sessionId → summary（简化后的） */
    sessions: new Map(),
    /** 按工作区分组，key = cwd；未归属的用 __ungrouped__ */
    byWorkspace: new Map(),
    /** 最近一次全量拉取成功的时刻（epoch ms）。null = 从未成功。 */
    listSyncedAt: null,
    /** 上一次的错误，供 UI 决定要不要显示 */
    lastError: null,
  }

  // ══════════════════════════════════════════════════════════════════
  // RPC 封装
  // ══════════════════════════════════════════════════════════════════

  let rpcSeq = 0

  /**
   * 调一个 DSH RPC。
   *
   * ⚠️ 四个必须遵守的点（都是实测踩过的）：
   *  1. 信封里的 `method` 要写**全名** `session/list`，不是裸 `list`。
   *     写裸名会得到
   *     `method "list" does not match endpoint "session/list"`。
   *  2. 结果信封是三层 `result.ok` / `result.value`，不是 `{ value }`。
   *     写错一层会「安静地得到 undefined」，然后界面上是一份空列表
   *     ——看起来像「部署上没有会话」，而真相是解包写错了。
   *  3. **args 内部还有一层 `request`。**
   *     正确：`{ args: { request: { …实际参数… } } }`
   *     错误：`{ args: { …实际参数… } }` → 得到
   *       `missing "request"; unexpected "address", …`
   *     **唯一例外是 `session/list`**，它的参数键恰好叫 `_request`，
   *     且是平铺的：`{ args: { _request: {} } }`。
   *  4. `requestId`（session/prompt）由**服务端去重**，同一 id 重试安全。
   *     错一次会产生两条轮次，不可逆。
   *
   * @param {string} namespace 例如 'session'
   * @param {string} method 例如 'list'
   * @param {object} args **内层**参数（不要自己包 request，本函数会包）
   * @param {string} [argsKey] args 里的键名。默认 'request'；
   *        `session/list` 要传 '_request'。
   * @returns {Promise<unknown>} result.value
   * @throws  {Error} RPC 返回 ok:false，或网络失败
   */
  async function rpc(namespace, method, args, argsKey) {
    const key = argsKey || 'request'
    const res = await window.fetch(`/api/${namespace}/${method}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        type: 'client-request',
        rpcId: `qx-m${++rpcSeq}`,
        method: `${namespace}/${method}`,   // ⚠️ 全名，不是裸 method
        payload: { args: { [key]: args } },
      }),
    })
    if (!res.ok) throw new Error(`HTTP ${res.status} ${res.statusText}`)
    const body = await res.json()
    const result = body && body.result
    if (!result) throw new Error('响应里没有 result 字段')
    if (result.ok !== true) {
      const err = result.error || {}
      throw new Error(`${err.code || 'unknown'}: ${err.message || JSON.stringify(err)}`)
    }
    return result.value
  }

  /**
   * `session/list`。⚠️ 它是**唯一**平铺的方法，参数键叫 `_request`。
   */
  function listSessions() {
    return rpc('session', 'list', {}, '_request')
  }

  /**
   * `session/page`。⚠️ `throughSeq` 是**游标**不是任意上限 ——
   * 填 `2**31-1` 会得到
   * `session page through seq 2147483647 is past cursor 13487`。
   * 真实值取自 `projections.asOfSeq`。
   */
  function pageSession(sessionId, asOfSeq, maxMessages) {
    return rpc('session', 'page', {
      address: { kind: 'session', sessionId },
      throughSeq: asOfSeq,
      maxMessages: maxMessages || 50,
    })
  }

  // ══════════════════════════════════════════════════════════════════
  // 稀疏行认知 —— 这一版的重点
  // ══════════════════════════════════════════════════════════════════

  /**
   * `projections.values` **按签名分组**，不是固定结构。
   * 全量 394 条实测：{只有 title: 208, 18 字段: 65, 19 字段: 37, 20 字段: 84}
   *
   * ⚠️ 那 208 条**没有** `contextPressure` 键 —— 不是 null，不是 {x:0}，
   * 是键根本不存在。按固定结构解析并给缺失值兜底 0，会给一半以上的
   * 会话画出假的「上下文 0%」和假的「已完成」。
   *
   * 所以每个字段的读取都必须区分三种态：
   *   键存在且有值 → 用它
   *   键存在但为 null → 显示「无」（不是 0）
   *   键不存在    → **整块不显示**（不是 0）
   */
  function readProjection(item) {
    const values = (item && item.projections && item.projections.values) || {}
    const has = (key) => Object.prototype.hasOwnProperty.call(values, key)

    // 只有 title 的会话 = 从未跑过一轮（无 sessionStats / turnOutline）
    const sparse = !has('sessionStats') && !has('turnOutline')

    const stats = has('sessionStats') ? values.sessionStats : null
    const outline = has('turnOutline') ? (values.turnOutline || []) : null
    const pressure = has('contextPressure') ? values.contextPressure : null
    const timing = has('subagentTiming') ? values.subagentTiming : null
    const team = has('agentTeam') ? values.agentTeam : null
    const questions = has('userQuestions') ? values.userQuestions : null

    // 上下文百分比：只能用 projectedTokens / contextWindow
    // （只有它给分母）。⚠️ 缺失时返回 null，调用方必须整块不显示，
    // **不得兜底成 0**。
    const contextPct = pressure && pressure.contextWindow
      ? (pressure.projectedTokens / pressure.contextWindow) * 100
      : null

    return {
      sessionId: item.sessionId,
      title: values.title || '（无标题）',
      cwd: item.cwd || '',
      origin: item.origin || null,
      parentSessionId: item.parentSessionId || null,
      running: item.running === true,
      updatedAt: item.updatedAt || 0,
      sparse,
      turns: outline ? outline.length : null,
      steps: stats && typeof stats.steps === 'number' ? stats.steps : null,
      contextPct,
      // lastTurnCompleted 只在有 subagentTiming 时才有意义
      lastTurnCompleted: timing ? timing.lastTurnCompleted === true : null,
      teamSize: team && team.members ? team.members.length : null,
      // ⚠️ `questions === null` 是「不可知」，**不是**「没有待答」。
      //    混淆两者会违反「否定断言律」：没收到 ≠ 不存在。
      questionsKnown: questions !== null,
      hasPendingQuestion: questions ? (questions.active || []).length > 0 : null,
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
    // 组内按 updatedAt 倒序；空组排最后
    return map
  }

  async function refreshSessions() {
    try {
      const value = await listSessions()
      const items = (value && value.items) || []
      state.sessions.clear()
      const list = []
      for (const item of items) {
        const parsed = readProjection(item)
        state.sessions.set(parsed.sessionId, parsed)
        list.push(parsed)
      }
      state.byWorkspace = groupByWorkspace(list)
      state.listSyncedAt = Date.now()
      state.lastError = null
      return { count: list.length, sparse: list.filter((x) => x.sparse).length }
    } catch (error) {
      state.lastError = String(error)
      throw error
    }
  }

  // ══════════════════════════════════════════════════════════════════
  // 诊断浮层
  // ══════════════════════════════════════════════════════════════════

  async function mount() {
    let summary = null
    let error = null
    try {
      summary = await refreshSessions()
    } catch (cause) {
      error = String(cause)
    }

    const root = document.createElement('div')
    root.dataset.qxMobileRoot = ''
    root.style.cssText = [
      'position:fixed', 'z-index:2147483647',
      'left:8px', 'right:8px', 'top:calc(env(safe-area-inset-top) + 8px)',
      'max-height:70vh', 'overflow:auto',
      'background:#0b0e13', 'color:#e8edf2',
      'border:1px solid rgba(255,255,255,.16)', 'border-radius:12px',
      'padding:12px 14px', 'font:12px/1.6 ui-monospace,Menlo,monospace',
      'box-shadow:0 12px 40px rgba(0,0,0,.5)', 'pointer-events:auto',
    ].join(';')

    const lines = []
    const add = (ok, text) => lines.push({ ok, text })

    if (error) {
      add(false, `拉取失败：${error}`)
      add(false, '说明通道未通 —— 检查：① WebView 是否带了 qx_token cookie ② 网关是否在跑')
    } else {
      add(true, `全量拉取成功：${summary.count} 条`)
      add(summary.sparse > 0, `稀疏行（只有 title）：${summary.sparse} 条（实测应约 52.8%）`)

      const withCtx = [...state.sessions.values()].filter((x) => x.contextPct !== null)
      add(true, `带上下文：${withCtx.length} 条`)
      if (withCtx.length > 0) {
        const pcts = withCtx.map((x) => x.contextPct).sort((a, b) => a - b)
        const at = (p) => pcts[Math.min(pcts.length - 1, Math.floor(pcts.length * p))]
        add(true, `上下文 p50 ${at(0.5).toFixed(1)}% · p90 ${at(0.9).toFixed(1)}% · max ${pcts[pcts.length - 1].toFixed(1)}%`)
        add(true, `>50%: ${pcts.filter((p) => p > 50).length} 条 · >65%: ${pcts.filter((p) => p > 65).length} 条`)
      }

      const ws = [...state.byWorkspace.entries()]
      add(ws.length > 0, `工作区分组：${ws.length} 个`)
      for (const [key, rows] of ws.slice(0, 6)) {
        const name = key === '__ungrouped__' ? '（未归属）' : key.split(/[\\/]/).pop()
        add(true, `  ${name}  ${rows.length} 条 · 稀疏 ${rows.filter((r) => r.sparse).length}`)
      }

      const pending = [...state.sessions.values()].filter((x) => x.hasPendingQuestion === true)
      const unknown = [...state.sessions.values()].filter((x) => x.questionsKnown === false)
      add(true, `有待答：${pending.length} 条 · 不可知：${unknown.length} 条`)
      add(pending.length === 0 && unknown.length > 0,
        '⚠️ 「有待答 0」不能断言「没有待答」—— 不可知的要单列')
    }

    const pass = lines.filter((l) => l.ok).length
    const fail = lines.length - pass

    const head = document.createElement('div')
    head.style.cssText = 'font-weight:700;font-size:13px;margin-bottom:8px;color:'
      + (fail === 0 ? '#3DDC97' : '#FFC24B')
    head.textContent = `千寻 L3 第 1 版 · 通过 ${pass} · 注意 ${fail}`
    root.append(head)

    for (const line of lines) {
      const row = document.createElement('div')
      row.style.cssText = 'display:flex;gap:6px;padding:3px 0;border-top:1px solid rgba(255,255,255,.07);word-break:break-all'
      const mark = document.createElement('span')
      mark.style.cssText = 'flex:0 0 auto;font-weight:700;color:'
        + (line.ok ? '#3DDC97' : '#FFC24B')
      mark.textContent = line.ok ? '✓' : '!'
      const text = document.createElement('span')
      text.textContent = line.text
      row.append(mark, text)
      root.append(row)
    }

    const actions = document.createElement('div')
    actions.style.cssText = 'display:flex;gap:8px;margin-top:10px'

    const refreshBtn = document.createElement('button')
    refreshBtn.textContent = '重新拉取'
    refreshBtn.style.cssText = 'flex:1;padding:9px;border-radius:8px;border:1px solid rgba(255,255,255,.2);background:#1a2029;color:#e8edf2;font:600 13px system-ui'
    refreshBtn.onclick = async () => {
      refreshBtn.textContent = '拉取中…'
      try {
        const s = await refreshSessions()
        refreshBtn.textContent = `已更新 ${s.count} 条`
      } catch (cause) {
        refreshBtn.textContent = `失败：${cause}`.slice(0, 40)
      }
    }

    const closeBtn = document.createElement('button')
    closeBtn.textContent = '关闭'
    closeBtn.style.cssText = 'flex:1;padding:9px;border-radius:8px;border:1px solid rgba(255,255,255,.2);background:#1a2029;color:#e8edf2;font:600 13px system-ui'
    closeBtn.onclick = () => root.remove()

    actions.append(refreshBtn, closeBtn)
    root.append(actions)

    document.body.append(root)
    window.console.log('[qx-mobile L3 v1]', lines, state)

    // 40 秒后自动收起 —— 它是诊断工具，不该长期占屏
    const timer = window.setTimeout(() => root.remove(), 40000)
    return () => {
      window.clearTimeout(timer)
      root.remove()
    }
  }

  return mount()
})()
