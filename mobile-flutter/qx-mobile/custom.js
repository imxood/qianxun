/*
 * 千寻移动端 · L3 定制脚本（custom.js） —— 第 3 版
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
 * 要等网络返回才跑 —— 于是界面永远不出现。
 *
 * 恶劣之处：**没有任何报错**。bootstrap 只在 catch 里打 console，
 * 而它 catch 的是同步异常；一个「类型不对但没抛」的返回值一路静默通过。
 * 唯一线索是「界面没动」，极易误判成 CSS 没生效。
 *
 * 正确：register(async function qxMount(ctx) { ... })
 *
 * ============================== 第 4 版相对第 3 版的改动 ==============================
 *
 * ⚠️ **一、DOM 行 → sessionId 的映射找到了（第 3 版最大的将就）。**
 *   真机实测：`data-row-key` 就是那张映射表：
 *       工作区行  data-row-key="workspace:48231145-4874-434e-b47c-06070bff3668"
 *       会话行    data-row-key="session:session-776fc1e4-480b-4e49-82b7-f8ff63602092"
 *   于是第 3 版的「按标题匹配」整个删掉，三个缺陷一并消失：
 *     · 标题被服务端截断到 40 字，DOM 里还可能带 markdown → 文本对不齐
 *     · 实测 4 条标题**完全相同** → 只能取「最近更新的那条」，是**猜**
 *     · 12 字前缀在中英文混排下会误匹配
 *   ⚠️ 之前判定「拿不到」是因为只在 role / aria-* / class 里找。
 *      可见锚点清单必须定期重扫 —— 旧清单不代表没有新锚点。
 *
 * ⚠️ **二、Preview Notice 的抑制修好了（第 3 版那条从来没生效过）。**
 *   第 3 版在 `div,section,aside,dialog` 里找**最小**的含文案容器，
 *   遇到「子节点也含同样文案」就跳过、交给子节点处理。
 *   但含文案的叶子是 `<p>` —— **不在那个标签列表里**，
 *   于是 root/dialog/inner 三层 div 全被跳过，一个都没隐藏。
 *   真机症状：hidden=0、弹窗照旧，看起来像「代码没执行」。
 *   它是模态框，吞掉所有点击 —— 侧栏点不动、截图永远停在它上面。
 *
 * ⚠️ **三、侧栏展开后自动收起。**
 *   窄屏下侧栏是全屏浮层（见 custom.css §A1）。真机实测：
 *   点会话**确实会跳转**，但侧栏**不收起** → 用户永远看不到对话。
 *   桌面是三栏并排，「选完收起」没意义，DSH 当然不做 —— 得我们做。
 *   做法：捕获阶段监听 click，命中**会话行**（data-row-key 以 session: 开头）
 *   就延迟一帧后点 DSH 自己的「Collapse sidebar」，
 *   不自己改 DOM 状态（会和它的内部状态机打架）。
 *
 * ============================== 第 3 版做过的事（第 4 版保留）==============================
 *
 * ⚠️ **一、撤掉第二套导航。**
 *   第 2 版加了 ☰ 全屏抽屉，与 DSH 自带的 56px rail **撞成两套导航**。
 *   用户不知道该点哪个，DSH 的 rail 也被挡住。
 *   现在**一套导航**：DSH 自己的 rail（窄屏压成图标列是合理的，
 *   dsh-mobile 也这么做），点它的「工作区」图标就展开工作区树。
 *   本文件不再创建任何按钮、抽屉、遮罩或手势。
 *
 * ⚠️ **二、工作区归属改用注册表，不再按 cwd 猜。**
 *   第 2 版按 `session/list` 的 `cwd` 分组，造出 8 个「工作区」。
 *   实测（scripts/probe_workspace_ownership.py）：
 *       session/list 总数              394
 *       注册表登记的会话数            143
 *       **未登记（DSH 里的「未分组」）** 251  (63.7%)
 *   注册表在 `<dsh_home>/storages/workspace.json`：
 *       tables.workspaces[id] = { path, title, sessionIds[], createdAt, updatedAt }
 *       global = { workspaceIds[6], archivedSessionIds[3], pinnedSessionIds[] }
 *   所以真实结构是 **6 个工作区 + 1 个未分组区**（与 DSH 侧栏一致），
 *   不是 8 个。按 cwd 分组等于在手机上**伪造 DSH 里不存在的层级**。
 *
 *   ⚠️ 注册表**没有 RPC**（实测 /api/workspace/* 全 404），而 DSH 页面
 *      内部有 workspaceController。所以归属只能**从页面 DOM 读** ——
 *      见 paintSidebarRows()。
 *
 * ⚠️ **三、未分组 251 条：默认收起，且用 DSH 原生措辞。**
 *   ⚠️⚠️ **第 4 版实测推翻了自己的前提**：DSH 侧栏里**根本没有**「未分组」组
 *      （全页扫 `未分组|Ungrouped` 叶子节点 = 0 个）。
 *      也就是说这 251 条在**桌面端也只能靠搜索**到达。
 *      手机端是否要补这个组，属于产品决策，不是本文件能定的 ——
 *      见 docs/design/13-窄屏侧栏决策.md。
 *      WORD.ungrouped 等常量先留着，用到时才接上。
 *   DSH 的 i18n（实测自 10.6MB plugin chunk，见 probe_workspace_i18n.py）：
 *       group.ungrouped  = 未分组 / Ungrouped
 *       row.archived     = 已归档 / Archived
 *       row.pinned       = 已置顶 / Pinned
 *       row.running      = 运行中 / Running
 *       section.workspaces = 工作区 / Workspaces
 *   **必须复用同一套措辞** —— 桌面写「未分组」手机写「其他会话」，
 *   用户会以为是两回事。
 *
 * ============================== 另外两条不能违反的律 ==============================
 *
 * ① 选择器只能用 data-* / role / aria-*。
 *    DSH 的 class 是**构建期哈希**（实测 hHd-Xa_panelRow / bhn1Oq_searchButton /
 *    YDXeBa_projectRow），每次 DSH 构建都变。清单见 docs/probe/dom-anchors.json。
 *
 * ② 不隐藏 DSH 原版界面，也不重写转录。
 *    `.dshm-shell { display:none }` 是「重写」不是「重排」，
 *    会把 141 个 RPC 的 UI 工作量全揽过来。转录是 DSH 的核心价值。
 *
 * ============================== 线格式（全部实测）==============================
 *   POST /api/<ns>/<method>
 *   method: **全名** `session/list`（写裸 `list` 会得到
 *           method "list" does not match endpoint "session/list"）
 *   payload.args: **内层还有一层 request**
 *     { args: { request: { …实际参数… } } }
 *     唯一例外 session/list：平铺，键名恰好叫 `_request`
 *   响应: { type:'server-response', rpcId, result:{ ok, value|error } }
 */

window.qxMobile.register(async function qxMount({ document: doc, window: win }) {

  // ══════════════════════════════════════════════════════════════════
  // 常量
  // ══════════════════════════════════════════════════════════════════

  /** 工具输出折叠阈值。略高于实测 p90（9.4KB）。 */
  const FOLD_BYTES = 12 * 1024

  /**
   * DSH 的原生措辞（实测自 10.6MB plugin chunk 的 i18n 字典）。
   * ⚠️ 不自己造词 —— 桌面与手机用同一套，用户才不会以为是两回事。
   */
  const WORD = {
    ungrouped: '未分组',
    archived: '已归档',
    pinned: '已置顶',
    running: '运行中',
    workspaces: '工作区',
    sessions: '会话',
    pending: '待审批',
    idle: '空闲',
    done: '已完成',
    settled: '已收工',
    unclear: '状态不明',
    neverRan: '还没有发过消息',
    fullText: '看全文',
    about: '约',
    k: 'K',
  }

  // ══════════════════════════════════════════════════════════════════
  // 状态
  // ══════════════════════════════════════════════════════════════════

  const state = {
    sessions: new Map(),      // sessionId → summary
    listSyncedAt: null,
    lastError: null,
    /** DSH 侧栏的 treeitem 元素 → 我们的补充信息（弱引用，随 DOM 消失） */
    rowInfo: new WeakMap(),
    /** 已加过标记的行，避免重复插 */
    painted: new WeakSet(),
    pollTimer: 0,
    /** 选完会话自动收起侧栏的监听器（dispose 时要摘掉） */
    onRowPicked: null,
  }

  let rpcSeq = 0
  let layer = null

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

  // ══════════════════════════════════════════════════════════════════
  // 摘要解析
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

    const sparse = !has('sessionStats') && !has('turnOutline')
    const stats = has('sessionStats') ? values.sessionStats : null
    const outline = has('turnOutline') ? (values.turnOutline || []) : null
    const pressure = has('contextPressure') ? values.contextPressure : null
    const timing = has('subagentTiming') ? values.subagentTiming : null
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
      running: item.running === true,
      updatedAt: item.updatedAt || 0,
      sparse,
      pendingKind,
      turns: outline ? outline.length : null,
      steps: stats && typeof stats.steps === 'number' ? stats.steps : null,
      contextPct,
      lastTurnCompleted: timing ? timing.lastTurnCompleted === true : null,
      lastResponse: outline && outline.length > 0
        ? (outline[outline.length - 1].response || '')
        : null,
    }
  }

  async function refreshSessions() {
    const value = await listSessions()
    const items = (value && value.items) || []
    state.sessions.clear()
    for (const item of items) {
      const parsed = readProjection(item)
      state.sessions.set(parsed.sessionId, parsed)
    }
    state.listSyncedAt = Date.now()
    state.lastError = null
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
    for (const child of [].concat(children || [])) {
      if (child === null || child === undefined || child === false) continue
      node.appendChild(typeof child === 'string' ? doc.createTextNode(child) : child)
    }
    return node
  }

  const textOf = (node) => (node ? (node.textContent || '').trim() : '')

  const note = (message) => {
    if (win.console) win.console.log('[qx-mobile]', message)
  }

  // ══════════════════════════════════════════════════════════════════
  // DSH 的 Preview Notice —— 必须处理，否则一切验证都做不了
  // ══════════════════════════════════════════════════════════════════

  /**
   * 关掉 DSH 的「Preview Notice」模态框。
   *
   * 为什么非做不可：它每次页面重载都弹，而它是**模态**的 ——
   * 吞掉所有点击。于是「展开工作区侧栏」点不动、截图永远停在这个框上，
   * 每部署一次 custom.js 就要手点一次 Continue。
   * 看起来像「我们的代码没生效」，其实是别人的弹窗在挡。
   *
   * 为什么在 JS 里做而不是改它的存储：
   *   · 扫过 `<dsh_home>` 的全部 json —— **没有** preview/notice 相关键
   *   · 扫过全部 4 个 JS 资源（合计 13MB）的 i18n 字典 —— **也没有**
   *   · 说明它的「已看过」状态在浏览器侧（localStorage/IndexedDB），
   *     或者压根就没记（每次必弹）
   *   既然找不到，就从渲染层解决。
   *
   * 做法：**隐藏而非删除**。删节点可能触发它的重渲染逻辑，
   * 那就会变成「删了又出来」的循环。隐藏是幂等的。
   *
   * ⚠️ 第 3 版修掉一个「跑了但永远命中 0 个」的写法（真机实测）：
   *   旧写法在 `div, section, aside, dialog` 里找**最小**的含文案容器，
   *   遇到「子节点也含同样文案」就跳过、交给子节点处理。
   *   但含文案的叶子是 `<p>`/`<h2>` —— **不在那个选择器里**，
   *   于是 3 个候选 div 全被 hasChild 跳过，一个都没隐藏。
   *   症状：`hidden=0`、弹窗照旧，看起来像「代码没执行」。
   *
   * ⚠️ 为什么不能隐藏它所在的父容器（真机实测结构）：
   *     DIV._root_o6lrb_6          ← 通用 Dialog 宿主
   *       ├── DIV._mask_o6lrb_18    position:absolute，空文案，吞点击
   *       └── DIV._dialog_o6lrb_33 position:relative, z=1，面板
   *   三个类名共用模块哈希 o6lrb —— 它是 DSH UI 套件的**通用弹窗容器**，
   *   审批/提问卡片也走这里。隐藏它 = 顺手把手机端最关键的审批弹窗也废了。
   *   所以只隐藏「文案匹配的那一个 dialog」+「它的遮罩兄弟」。
   */
  function suppressPreviewNotice() {
    const marks = ['Preview Notice', 'still in preview', '预览版']
    let found = 0
    const dialogs = doc.querySelectorAll('[role="dialog"], [aria-modal="true"]')
    for (let i = 0; i < dialogs.length; i += 1) {
      const dialog = dialogs[i]
      const t = dialog.textContent || ''
      let matched = false
      for (let m = 0; m < marks.length; m += 1) {
        if (t.indexOf(marks[m]) !== -1) { matched = true; break }
      }
      if (!matched) continue

      dialog.style.setProperty('display', 'none', 'important')
      dialog.setAttribute('data-qx-hidden-notice', '')

      // 遮罩是它的兄弟节点且**不含文案**（所以旧写法永远看不到它）。
      // 不隐藏它的话面板没了、底下照样点不动 —— 这才是「吞点击」的本体。
      const host = dialog.parentElement
      if (!host) continue
      const siblings = host.children || []
      for (let s = 0; s < siblings.length; s += 1) {
        const sib = siblings[s]
        if (sib === dialog) continue
        const pos = win.getComputedStyle(sib).position
        if (pos === 'absolute' || pos === 'fixed') {
          sib.style.setProperty('display', 'none', 'important')
          sib.setAttribute('data-qx-hidden-mask', '')
        }
      }
      found += 1
    }
    return found
  }

  // ══════════════════════════════════════════════════════════════════
  // 给 DSH 自己的侧栏行补手机端信息
  // ══════════════════════════════════════════════════════════════════

  /**
   * 加工 DSH 已经渲染出来的会话行，**不新增任何行**。
   *
   * 为什么改它而不是自己画：
   *   · DSH 的行已经带了正确的工作区归属（那是它的 workspaceController）
   *   · 自己画就得自己算归属，而注册表没有 RPC（实测 /api/workspace/* 全 404）
   *   · 自己画还会与 DSH 的行并存 —— 那就是「两套会话列表」
   *
   * 补什么（都是 DSH 侧栏**没有**、而手机端**必须**的）：
   *   1. **稀疏行形态** —— 208/394 的会话 `projections.values` 只有 title，
   *      DSH 自己无从判断「它没跑过」。我们补一句「还没有发过消息」，
   *      免得用户以为加载失败。
   *   2. **待答三档** —— 「有待答 / 确定没有 / 无法确认」分开说。
   *   3. **上下文占用** —— >50% 才显示（实测 p90=53.2%）。
   *
   * 怎么认出行：DSH 的树用 `role="treeitem"`，行上带 `data-row-key`
   * （实测格式见下面的 sessionIdOfRow）。
   */
  function paintSidebarRows() {
    let items
    try {
      items = doc.querySelectorAll('[role="treeitem"]')
    } catch (error) {
      return
    }

    for (let i = 0; i < items.length; i += 1) {
      const row = items[i]
      if (state.painted.has(row)) continue
      state.painted.add(row)

      // 工作区行没有 sessionId，直接跳过（不加工）
      const summary = findByRow(row)
      if (!summary) continue

      // 1. 稀疏行：补一句「还没有发过消息」
      if (summary.sparse) {
        const hint = el('div', {
          class: 'qx-hint-sparse',
          text: WORD.neverRan,
        })
        row.appendChild(hint)
      }

      // 2. 上下文占用（>50% 才显示；>65% 转琥珀）
      if (summary.contextPct !== null && summary.contextPct > 50) {
        row.appendChild(el('div', {
          class: 'qx-hint-ctx' + (summary.contextPct > 65 ? ' qx-hint-ctx--hi' : ''),
          text: '上下文 ' + Math.round(summary.contextPct) + '%',
        }))
      }

      // 3. 待答：只在真的有待答时加（不可知的不加标签 ——
      //    那是「不可知」，不是「待答」；不可知的统计放在侧栏底部）
      if (summary.pendingKind === 'pending') {
        row.classList.add('qx-row--pending')
      }
    }
  }

  /**
   * 从 DOM 行取出它的 sessionId。
   *
   * ✅ **第 4 版找到了精确映射** —— `data-row-key`（真机实测）：
   *     工作区行  data-row-key="workspace:48231145-4874-434e-b47c-06070bff3668"
   *     会话行    data-row-key="session:session-776fc1e4-480b-4e49-82b7-f8ff63602092"
   *
   * 它一次解决了第 3 版三个已知缺陷：
   *   1. 标题被服务端截断到 40 字（实测 max=40），DOM 里还可能带 markdown 标记
   *      → 文本匹配根本对不齐
   *   2. 实测有 **4 条标题完全相同**（「我一般不看新闻 论坛 社交媒…」），
   *      第 3 版只能取「最近更新的那条」——那是**猜**，且在标题全同时会张冠李戴
   *   3. 12 字前缀会在中英文混排下误匹配
   *
   * ⚠️ 之前判定「拿不到映射」是因为只在 `role` / `aria-*` / class 里找，
   *    而 `data-row-key` 不属于这三种 —— 可见的锚点清单要**定期重扫**，
   *    旧清单不代表没有新锚点（docs/probe/dom-anchors.json 已补录）。
   */
  function sessionIdOfRow(row) {
    let node = row
    while (node && node !== doc.body) {
      const key = node.getAttribute && node.getAttribute('data-row-key')
      if (key && key.indexOf('session:') === 0) return key.slice('session:'.length)
      node = node.parentElement
    }
    return null
  }

  function findByRow(row) {
    const id = sessionIdOfRow(row)
    if (!id) return null
    return state.sessions.get(id) || null
  }

  // ══════════════════════════════════════════════════════════════════
  // 工具卡：只**加**入口，不重写组件
  // ══════════════════════════════════════════════════════════════════

  /**
   * 给 DSH 自己的工具行加「看全文」入口。
   *
   * 依据实测（docs/design/12-transcript-scale.md）：
   *   工具事件 p50 = 94 B（小）；消息 p50 = 6.9 KB（大）
   *   p90 = 9.4 KB  -> 阈值 12 KB
   *   单条最大 139 KB -> 139KB 塞进主视图会挤掉一切，必须进 sheet
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
    }
  }

  function attachFullButton(row) {
    if (row.querySelector('[data-qx-full]')) return
    const text = (row.textContent || '').trim()
    if (!text || text.length < 400) return
    row.appendChild(el('button', {
      class: 'qx-full-btn', type: 'button',
      'data-qx-full': '',
      text: WORD.fullText + '（' + WORD.about + ' '
        + Math.round(text.length / 1024) + WORD.k + '）',
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
        el('span', {
          class: 'qx-sheet-size',
          text: Math.round(text.length / 1024) + WORD.k,
        }),
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
  // 侧栏变浮层后的收尾：选完会话自动收起
  // ══════════════════════════════════════════════════════════════════

  /**
   * 真机实测（412px）：点会话**确实会跳转**（标题变成新会话名），
   * 但侧栏**不会自动收起** —— 而窄屏下侧栏是全屏浮层，
   * 于是用户点完会话看到的还是菜单，永远进不去对话。
   *
   * 为什么必须由我们来做：DSH 是按桌面三栏设计的，
   * 「选完就收起」在桌面没有意义（并排显示），它当然不会做。
   *
   * 做法：捕获阶段监听 click，命中**会话行**（data-row-key 以 session: 开头）
   * 就延迟一帧后点 DSH 自己的「Collapse sidebar」按钮 ——
   * 不自己改 DOM 状态，那会和它的内部状态机打架。
   *
   * 只在窄屏做：宽屏下侧栏是并排的，收起它反而是帮倒忙。
   */
  function installAutoCollapseSidebar() {
    const NARROW = 760
    state.onRowPicked = (event) => {
      if (win.innerWidth > NARROW) return
      const target = event.target
      if (!target || !target.closest) return

      // 只认**会话行**。点工作区行是展开/折叠，浮层下不需要收起。
      const row = target.closest('[role="treeitem"]')
      if (!row || !sessionIdOfRow(row)) return

      win.setTimeout(() => {
        const frame = doc.querySelector('[data-slot="root"] > *')
        if (!frame || frame.hasAttribute('data-sidebar-collapsed')) return
        const buttons = doc.querySelectorAll('[data-slot="sidebar"] button')
        for (let i = 0; i < buttons.length; i += 1) {
          const label = buttons[i].getAttribute('aria-label') || ''
          if (label.toLowerCase().indexOf('collapse sidebar') !== -1) {
            buttons[i].click()
            return
          }
        }
      }, 60)
    }
    doc.addEventListener('click', state.onRowPicked, true)
  }

  // ══════════════════════════════════════════════════════════════════
  // 启动
  // ══════════════════════════════════════════════════════════════════

  try {
    await refreshSessions()
  } catch (error) {
    state.lastError = String(error)
  }

  layer = el('div', { class: 'qx-layer', 'data-qx-layer': '' })
  layer.style.zIndex = '2147483644'
  doc.body.appendChild(layer)

  // DSH 每次渲染转录/侧栏都会换掉 DOM，所以要重扫。
  // 页面可见时才扫，代价极低；漏扫的行（没有状态提示）是用户看得见的缺陷。
  const scan = () => {
    // 弹窗可能在我们挂载**之后**才渲染（插件加载是异步的），
    // 所以每轮都查一遍。已隐藏的节点 style 已置位，重复设是幂等的。
    suppressPreviewNotice()
    paintToolCards()
    paintSidebarRows()
  }
  // 头 6 秒每 300ms 扫一次 —— 覆盖 DSH 插件异步加载那段窗口，
  // 否则用户会在弹窗消失前先摸到屏幕。
  let warmup = 0
  for (let i = 0; i < 20; i += 1) {
    warmup = win.setTimeout(() => { if (doc.visibilityState !== 'hidden') suppressPreviewNotice() }, 300 * (i + 1))
  }
  state.pollTimer = win.setInterval(() => {
    if (doc.visibilityState !== 'hidden') scan()
  }, 2000)

  installAutoCollapseSidebar()

  let pending = 0
  let unknown = 0
  state.sessions.forEach((s) => {
    if (s.pendingKind === 'pending') pending += 1
    if (s.pendingKind === 'unknown') unknown += 1
  })
  note('L3 第4版：' + state.sessions.size + ' 条会话，'
    + '待答 ' + pending + ' · 无法确认 ' + unknown
    + '，错误=' + (state.lastError || '无'))

  // ⚠️ 必须返回**函数**（清理用）。bootstrap.js:48 判 typeof。
  return function qxDispose() {
    if (state.pollTimer) win.clearInterval(state.pollTimer)
    if (warmup) win.clearTimeout(warmup)
    if (state.onRowPicked) doc.removeEventListener('click', state.onRowPicked, true)
    if (layer) { layer.remove(); layer = null }
    for (const node of doc.querySelectorAll('[data-qx-sheet]')) node.remove()
  }
})
