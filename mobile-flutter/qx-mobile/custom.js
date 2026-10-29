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
        appendHint(row, el('div', {
          class: 'qx-hint-sparse',
          text: WORD.neverRan,
        }))
      }

      // 2. 上下文占用：**不再插进列表**（用户明确要求）。
      //    它现在由 composer 工具栏里的「N%」标签承担 ——
      //    那个标签就在输入框旁边，看上下文是「准备发消息」时的动作；
      //    而列表是「找会话」时的浏览场景，两边都放是**重复信息**，
      //    而且列表行只有 48px，塞进去会把标题挤到只剩几十像素。
      //
      //    ⚠️ 原来那个标签还写在 `.qx-hint-ctx`，定位是 `position:absolute`
      //    压在标题右端 —— 实测它与标题**重叠 52px**，
      //    在 A22 把行宽收回视口内之后糊成一团。
      //    去掉它同时解决了「重复」和「重叠」两件事。
      //    保留：`.qx-hint-sparse`（解释「这一行为什么是空的」，别处没有）、
      //    `.qx-row--pending`（待答是跨会话的状态，列表里需要一眼看到）。

      // 3. 待答：只在真的有待答时加（不可知的不加标签 ——
      //    那是「不可知」，不是「待答」；不可知的统计放在侧栏底部）
      if (summary.pendingKind === 'pending') {
        row.classList.add('qx-row--pending')
      }
    }

    // 清掉**上一轮**插进去的上下文标签。
    // ⚠️ 必须每轮清而不是只在插入时清：`state.painted` 让同一行只加工一次，
    //    热替换之后新 mount 的 painted 是空的，会重插 —— 但老节点还在，
    //    不清就会出现两个（这是幂等测试当初抓的那一类）。
    const staleCtx = doc.querySelectorAll('.qx-hint-ctx')
    for (let s = 0; s < staleCtx.length; s += 1) staleCtx[s].remove()
  }

  /**
   * 插提示条，**先清掉同类旧节点**。
   *
   * ⚠️ 为什么不能只靠 `state.painted`（WeakSet）防重复：
   *   定制层是**热替换**的 —— bootstrap 轮询到 hash 变了就 dispose 旧的、
   *   mount 新的，而提示条是插在 **DSH 自己的行**里的，不在我们自己的
   *   layer 里。dispose 掉 layer 并不会带走它们。
   *   于是新 mount 的 `state.painted` 是**空**的，会再插一遍。
   *   真机症状：一行出现两个「上下文 79%」（第 4 版截图里实测到）。
   *
   * 所以：清掉同类的再插，幂等，不依赖 mount 之间共享状态。
   */
  function appendHint(row, node) {
    const old = row.querySelectorAll('.' + node.className.split(' ')[0])
    for (let i = 0; i < old.length; i += 1) old[i].remove()
    row.appendChild(node)
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
   * 找 DSH 侧栏的「展开 / 收起」按钮。
   *
   * ⚠️⚠️ **文案随 DSH 的界面语言变，这是一条真机上才暴露的坑：**
   *   模拟器上 DSH 是英文，按钮 aria-label = `Open sidebar` / `Collapse sidebar`，
   *   真机（中文）上是 `打开侧边栏` / `收起侧边栏`。
   *   只认英文的话，在真机上点 ☰ **完全没反应** ——
   *   而且界面上看不出任何异常，最容易误判成「按钮没插进去」。
   *   （这一条是同一批里第二个「模拟器正常、真机坏了」的坑，
   *     第一个是 CDP 连错 WebView。）
   *
   * 所以两种语言都认，且都找不到时**退回侧栏里的第一个按钮** ——
   * 折叠态侧栏的第一个按钮按设计就是切换键。
   */
  function findSidebarToggle(collapsed) {
    const want = collapsed
      ? ['打开侧边栏', 'open sidebar']
      : ['收起侧边栏', 'collapse sidebar', 'close sidebar']
    const buttons = doc.querySelectorAll(
      '[data-slot="sidebar"] button, [data-slot="sidebar"] [role="button"]')
    for (let i = 0; i < buttons.length; i += 1) {
      const label = (buttons[i].getAttribute('aria-label') || '').toLowerCase()
      for (let k = 0; k < want.length; k += 1) {
        if (label.indexOf(want[k].toLowerCase()) !== -1) return buttons[i]
      }
    }
    return buttons.length ? buttons[0] : null
  }

  /**
   * 真机实测（412px）：点会话**确实会跳转**（标题变成新会话名），
   * 但侧栏**不会自动收起** —— 而窄屏下侧栏是全屏浮层，
   * 于是用户点完会话看到的还是菜单，永远进不去对话。
   *
   * 为什么必须由我们来做：DSH 是按桌面三栏设计的，
   * 「选完就收起」在桌面没有意义（并排显示），它当然不会做。
   *
   * 做法：捕获阶段监听 click，命中**会话行**（data-row-key 以 session: 开头）
   * 就延迟一帧后点 DSH 自己的收起按钮 ——
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
        const btn = findSidebarToggle(false)
        if (btn) btn.click()
      }, 60)
    }
    doc.addEventListener('click', state.onRowPicked, true)
  }

  /**
   * 把「被省略号砍掉」的横向行标记出来，交给 CSS 改成换行。
   *
   * 起因（359dp 真机宽度实测）：composer 底部状态行三个药丸
   *   「11 轮 446 步·103 t…」少 63px
   *   「89.2M tok·缓存命中 98%」少 57px
   * 「缓存命中 98%」被砍成「98」是**信息丢失** —— 用户会以为缓存没命中。
   *
   * ⚠️ 为什么用「JS 打标记 + CSS 改样式」而不是直接写选择器：
   *   那一行的 class 是构建期哈希（实测 bOPqQW_ / uV2eYG_ / JObwrW_），
   *   每次 DSH 构建都变，写进 CSS 等于给自己埋一颗定时炸弹。
   *   JS 按**行为**（scrollWidth > clientWidth）找它，与命名无关。
   *
   * ⚠️ 为什么不直接把标签加长：那是改文案不是改布局。
   */
  function markClippedRows() {
    for (const el of doc.querySelectorAll('span, div')) {
      if (el.scrollWidth <= el.clientWidth + 2) continue
      const r = el.getBoundingClientRect()
      if (r.width < 24 || r.height > 40) continue
      let row = el.parentElement
      // 往上找到最近一个「横向排布子元素」的容器 —— 那才是该换行的行
      while (row && row !== doc.body) {
        const disp = win.getComputedStyle(row).display
        if (disp.indexOf('flex') !== -1 || disp === 'grid') break
        row = row.parentElement
      }
      if (row && row !== doc.body) row.setAttribute('data-qx-clipped', '1')
    }
  }

  /**
   * 标记「真的看不清」的文本（字号 < 12px），交给 CSS 托到 12px。
   *
   * 实测（359dp）：转录里的代码块 chrome（`_header_` / `_heading_` /
   * `_language_`）是 11px，个别 code 10.5px。代码是要**照着抄**的内容，
   * 比正文更不该小；12px 是这里定的下限。
   *
   * ⚠️ 只标记「用户看得见的文字节点」，不扫所有元素 ——
   *    扫全树会把 DSH 那些不可见的测量节点也标上，白白多几百个属性。
   *
   * ⚠️ **必须用内联样式，不能只靠 CSS 规则。** 实测踩过：
   *    `._markdown_ :not(pre) > code { font-size: .875em }` 的特异性是
   *    (0,3,0) 且带 !important，压过了我们的 `[data-qx-tiny]`
   *    ——(0,1,0)+!important。标记打了，字号纹丝不动（11.375px）。
   *    内联 style 才能稳赢（除非对方也写内联 !important，实测没有）。
   */
  function markTinyText() {
    for (const el of doc.querySelectorAll('span, div, pre, code, kbd, samp')) {
      if (!el.firstChild) continue
      const txt = (el.textContent || '')
      if (!txt.trim()) continue
      if (parseFloat(win.getComputedStyle(el).fontSize) >= 12) continue
      el.setAttribute('data-qx-tiny', '1')
      el.style.setProperty('font-size', '12px', 'important')
    }
  }

  /**
   * 给 composer 底部那一行「统计药丸」打标记，交给 A17 重新分配宽度。
   *
   * 🔴 为什么需要单独一条：markClippedRows 只标记了**药丸自己**，
   *    它解决的是药丸**内部**的截断（「缓存命中 98%」被砍成「98」）。
   *    但药丸**之间**的宽度分配是父行决定的 —— A13 完全没碰过它，
   *    于是换行后的文字从药丸里溢出去，压到了下一个药丸上
   *    （实测重叠 36px，截图上「103」与「89.2M tok」糊成一团）。
   *
   * ⚠️⚠️ **第一版写错了，而且错得很隐蔽**：只用「横排容器 + 多个被截断
   *    的药丸」这个启发式，一次标上了 **45 个元素** ——
   *    转录里每一行 Think / Pwsh / Grep 都满足「横排 + 内部有截断」，
   *    于是 A17 的 `[data-qx-status] [data-qx-clipped] > *`
   *    规则会把**整个转录**的排版一起改掉。
   *
   *    这类规则一旦错标，症状是「页面整体变得很怪」而不是「某处不对」，
   *    极难往回找。**教训：标记类启发式必须有一个「只可能在这里出现」的锚。**
   *
   * 现在的锚：`[data-slot="conversation.composer.dock"]` —— composer 的底部停靠区，
   * 语义化、带版本，且只有那一行统计药丸在里面。
   * 实测祖先链：药丸 → 行 → [slot=conversation.composer.dock]（display:contents）。
   */
  /**
   * 找出 DSH 自己的统计按钮（轮数/步数/tok/缓存/上下文占用）。
   *
   * ⚠️⚠️ **这一版不再依赖 `[data-qx-clipped]` 标记**，前两版都在这里栽了：
   *   v1「药丸的 parentElement 就是行」→ 标到了**药丸按钮**上，
   *      A17 的「行铺满 / 药丸均分」全打进了药丸内部；
   *   v2「数直接子级里带 clipped 的」→ 一个都数不到，
   *      因为带标记的是**孙级 BUTTON**，标记整个消失。
   *
   * 根因：markClippedRows 标记的是「**被文字撑破**的元素」，
   * 而统计行的层级是 DIV[行] → SPAN[药丸壳] → BUTTON[带标记] → SPAN[文字]，
   * 层级比想的多一层。**依赖另一条启发式的产物，就等于继承它的所有误判。**
   *
   * 现在改成**独立判据**：文本里带统计关键词的按钮。
   * 它不依赖任何别处的标记，也不会随那些标记的实现变化而失效。
   */
  const STAT_RE = /turn|step|tok|cache|%|轮|步|命中|用量/i

  function findStatButtons() {
    const bar = doc.querySelector('[data-slot="conversation.composer.bar"]')
    if (!bar) return []
    const out = []
    for (const b of bar.querySelectorAll('button, [role="button"]')) {
      // ⚠️⚠️ **必须先排掉我们自己的按钮。**
      //   它的文字是 `79%`，同样命中 STAT_RE；
      //   于是它被当成 DSH 的统计按钮，一路把 findStatRow 带到了
      //   composer 的根 —— B12 就会把**整个输入框**收起来。
      //   症状是「按钮和输入框一起消失」，看着像页面崩了。
      //
      //   **规律：自己造的元素绝不能进入「找 DSH 元素」的判据。**
      if (b.closest('.qx-status-btn')) continue
      const r = b.getBoundingClientRect()
      // ⚠️ **只查宽度，不查高度。**
      //   状态区是被 `height:0 + visibility:hidden` 收起来的
      //   （见 custom.css B12：不能用 display:none，否则读不到数），
      //   它的按钮高度仍然是真实的，但万一 DSH 哪天把行压扁了，
      //   也不该让我们丢掉唯一的数据源。
      if (r.width < 1) continue
      const t = (b.textContent || '').trim()
      if (!t || !STAT_RE.test(t)) continue
      out.push(b)
    }
    return out
  }

  /**
   * 统计区 = composer 根底下、**与输入卡片并列的那个兄弟节点**。
   *
   * ⚠️ 前三版都栽在「数统计按钮」上：
   *   v1 依赖 [data-qx-clipped] 标记 → 标到了药丸按钮上
   *   v2 数直接子级里带标记的 → 一个都数不到（标记在孙级）
   *   v3 数「直辖 ≥2 个统计按钮的祖先」→ 一旦 B12 把药丸藏起来，
   *      它们 `getBoundingClientRect()` 变成 0×0 被过滤掉，
   *      计数随之变化，标记**跟着漂移** —— 实测药丸藏了、79% 圆环留在屏幕底部。
   *
   * 根因：**判据依赖了自己隐藏掉的那个元素**。隐藏是单向不可逆的，
   * 任何「数一数还剩几个」的判据在隐藏之后都会自己失效。
   *
   * 现在改成**位置判据**：从统计按钮往上走，走到「它的父节点里已经有输入区、
   * 而它自己没有」为止 —— 那就是统计区。
   * 这个判据只看结构、不看数量，隐藏前后都成立。
   */
  function findStatRow() {
    for (const b of findStatButtons()) {
      let n = b
      for (let i = 0; i < 6 && n && n.parentElement; i += 1, n = n.parentElement) {
        const p = n.parentElement
        if (!p.querySelector('[data-slot="conversation.input.left"]')) continue
        if (n.querySelector('[data-slot="conversation.input.left"]')) return null  // 它自己就是输入区
        return n
      }
    }
    return null
  }

  function markStatusRow() {
    const row = findStatRow()
    if (!row) return 0
    row.setAttribute('data-qx-status', '1')
    return 1
  }

  /**
   * 标记「太矮的块级可点元素」，交给 CSS 补到 40px 高。
   *
   * 实测（359dp）：转录里的审批/提问选项只有 23px 高、283px 宽 ——
   * 手指点「批准」这种一次误触就出事的地方，23px 是不够的。
   *
   * ⚠️ **不能一刀切所有矮元素**。正文里还有 15px 高的行内按钮
   *    （`O_Ebla_title` 134×15），那是句子里的链接；
   *    给它 `min-height:40px` 会把整段文字撑成一片，**比原来更糟**。
   *    所以三个条件缺一不可：
   *      1. display 是 block/flex（排除行内）
   *      2. 宽度 > 视口的 55%（行内链接不会占这么宽）
   *      3. 高度在 12–32px 之间（低于 12 的是图标碎片，高于 32 的本来就够）
   */
  // ── 第 10 轮：Trajectory 页左侧的「请求标记」是 16×16 ────────────
  //
  // 实测（359dp 视口）：
  //   BUTTON  16×16   透明背景、无子元素、无边框
  //     ::before   5×5 灰点  ← 唯一可见的东西
  //     ::after   "Request #1" 54×12  白底  ← 悬停标签
  //   一共 14 个，纵向相邻间距 60–106px（最小 60）。
  //
  // 16px 在手机上**永远点不准**，而它是「跳转到某次请求」的唯一入口。
  //
  // 为什么按钮本身放大是安全的：**圆点是 ::before 伪元素，不是子元素** ——
  // 按钮自身完全透明，放大只会让热区变大，那个 5px 的点不动。
  //
  // 用**对称负边距**把布局盒子拉回原尺寸，所以：
  //   · 行高不变（TR 仍 30px），滚动条长度不变
  //   · 圆点仍在原来的位置
  //   · 热区 16 → 44，且 44 < 60 的最小间距，装得下不重叠
  //
  // ⚠️ 判据写成「尺寸 + 无子元素 + 透明背景」，**不碰 DSH 的类名**：
  //    它是 `Y0dWHa_requestBoundaryControl`，哈希前缀每次构建都变。
  function widenDotTaps(session) {
    for (const el of session.querySelectorAll('button, [role="button"]')) {
      const r = el.getBoundingClientRect()
      // 只管「两个方向都 ≤20 的小方块」，且**没有子元素**（有子元素的
      // 是真按钮，缩小它会丢内容，不属于这一类）
      if (r.width > 20 || r.height > 20) continue
      if (el.children.length) continue
      const cs = win.getComputedStyle(el)
      // 透明背景才认 —— 有背景色的小块可能是色块本身（不该拉大）。
      // `transparent` 关键字和 rgba(...,0) 都算透明：
      // 规范上浏览器会归一化成 rgba(0,0,0,0)，但个别引擎会原样返回关键字，
      // 一行判断就能兜住，不值得为它冒「一个标记全都不动」的风险。
      const bg = cs.backgroundColor
      if (bg && bg !== 'transparent') {
        const m = /rgba?\(([^)]+)\)/.exec(bg)
        if (m) {
          const parts = m[1].split(',')
          const alpha = parseFloat((parts[3] !== undefined ? parts[3] : '1').trim())
          if (alpha > 0.02) continue
        } else {
          continue                                   // 认不出的颜色一律不动
        }
      }

      el.setAttribute('data-qx-dot-tap', '1')
      el.style.setProperty('min-width', '44px', 'important')
      el.style.setProperty('min-height', '44px', 'important')
      // 对称负边距：44 - w，两侧各退一半 → 布局盒子回到 w
      const dx = (44 - r.width) / 2
      const dy = (44 - r.height) / 2
      el.style.setProperty('margin-left', (-dx) + 'px', 'important')
      el.style.setProperty('margin-right', (-dx) + 'px', 'important')
      el.style.setProperty('margin-top', (-dy) + 'px', 'important')
      el.style.setProperty('margin-bottom', (-dy) + 'px', 'important')
    }
  }

  function markShortTapRows() {
    const vw = win.innerWidth
    for (const el of doc.querySelectorAll('button, [role="button"], [role="option"], a')) {
      const cs = win.getComputedStyle(el)
      const r = el.getBoundingClientRect()
      if (r.width < 24 || r.height < 12) continue

      if (cs.display === 'inline' || cs.display === 'inline-block'
          || cs.display === 'inline-flex') {
        // 行内元素（正文里的链接）：**纵向 padding 不改变行盒高度**，
        // 所以可以放心加大热区而不会把段落撑开 ——
        // 这与块级元素必须用 min-height 是两套办法，不能混用。
        if (r.height < 24) {
          el.setAttribute('data-qx-inline-tap', '1')
          el.style.setProperty('padding-top', '9px', 'important')
          el.style.setProperty('padding-bottom', '9px', 'important')
        }
        continue
      }

      if (cs.display !== 'block' && cs.display !== 'flex') continue
      // 阈值从 55% 降到 45%：实测文件链接 193/359 = 53.8%，
      // 差一点点就被 55% 挡在门外 —— 阈值卡太紧会漏掉真正该补的元素。
      let wideEnough = r.width >= vw * 0.45
      if (!wideEnough) {
        // 宽度不够时还有一条更准的结构判据：**父容器高度 ≈ 自己高度**，
        // 说明这个按钮就是整行（转录里工具/步骤的折叠行标题，实测
        // 「已读取文件」151×15、「已搜索代码并读取文件」151×15）。
        // 正文里的行内链接不会命中 —— 它的父容器是整段文字，
        // 高度远大于链接本身，给它补 44px 会把段落撑开。
        const p = el.parentElement
        const ph = p ? p.getBoundingClientRect().height : 0
        if (ph && Math.abs(ph - r.height) <= 3) wideEnough = true
        // ── 第 10 轮新增：工具条里的矮按钮 ──────────────────────────
        // Trajectory 页顶栏实测：
        //   DIV  容器 185×44（本来就 44 高）
        //   └ BUTTON "Collapse turns"  55×20   ← 只有 20 高，一半都不够
        //   └ BUTTON "Collapse calls" 51×20
        //
        // 原来的两条判据都匹配不上：宽度 55 < 0.45×359=162；
        // 父容器 44 与自身 20 相差 24 > 3。
        //
        // 补的判据：**父容器是 flex 行，且高度在 [自身高度, 60] 之间** ——
        // 即「父行就是一条工具条，它为别的按钮已经留够了 44 的空间」。
        // 上限 60 是必须的：正文段落、整块卡片父容器都远大于 60，
        // 不设上限就会把它们整段撑成 44 一行。
        //
        // ⚠️ 这一条**完全不用 DSH 的类名**：轨迹页的根节点没有 data-slot，
        //    而 CSS-module 的类名带构建期哈希前缀，靠它定位迟早失效。
        //    结构判据跨版本稳定，类名不。
        else if (ph && ph >= r.height && ph <= 60) {
          const pcs = win.getComputedStyle(p)
          if (pcs.display === 'flex' || pcs.display === 'inline-flex') wideEnough = true
        }
      }
      if (!wideEnough) continue
      if (r.height >= 44) continue
      el.setAttribute('data-qx-short-tap', '1')
      el.style.setProperty('min-height', '44px', 'important')
    }

    // ⚠️ 下面这段注释描述的是**一个已经被推翻的结论**，留着当反面教材：
    //
    //   「消息下方那排图标按钮（复制/重试/…）实测只有 24–27px 宽，
    //    但它们**间距只有 40px** —— 补到 44 会互相重叠，
    //    那是把「点不准」换成「点错地方」。所以取不重叠的 32×40。」
    //
    // 错在**把行内间距当成了可用宽度**。行距 8px、视口 359、
    // 一行只占 200 —— 补到 44 之后仍然只占 252，空得很。
    // 而「32×40 会不会重叠」这件事，**本来量一下就知道，不该推断**。
    //
    // ⚠️ 宽度 < 8 的要分两种（第一版把这里写反过，
    //    `if (r.width >= 8) continue` 会连 8…32 那一档一起跳过，
    //    结果触控过小从 6 个涨到 39 个 —— 分支顺序必须是「从小到大」）：
    //    (a) 里面装着 <img> 的 —— 那不是占位，是**塌掉的缩略图**，要救
    //    (b) 纯分隔/占位元素 —— 跳过
    const session = doc.querySelector('[data-slot="conversation.session"]')
    if (!session) return
    widenDotTaps(session)
    for (const el of session.querySelectorAll('button')) {
      const r = el.getBoundingClientRect()
      if (r.height < 20) continue
      if (r.width < 8) {
        // 🔴 实测（359dp）：转录里两张图片缩略图的按钮是 **1×44**，
        //   里面的 <img> 是 **0×43** —— 但它 `cursor:zoom-in`、
        //   有 onclick、tabIndex=0，是**真的能点的控件**。
        //   起因是那些 PNG 本身就是 1×1，DSH 按自然尺寸渲染就成了 1px。
        //   内容是 1×1 是事实，但**按钮只有 1px 宽就是缺陷**：
        //   手机上根本点不中，「点开看原图」这条路等于不存在。
        //   给一个最小可点尺寸，图片用 object-fit:contain 保持比例。
        if (el.querySelector('img')) {
          el.setAttribute('data-qx-thin-media', '1')
          el.style.setProperty('min-width', '44px', 'important')
          el.style.setProperty('min-height', '44px', 'important')
        }
        continue                                  // (b) 纯占位，跳过
      }
      // 🔴 上一轮我在这里写的是「间距只有 40px，补到 44 会互相重叠」，
      //   于是压到 32×40。**那个理由从一开始就是错的** ——
      //   我拿「行内间距」当成了「可用宽度」，而真正该问的是「这一行占了视口多少」。
      //
      //   实测（359dp 视口，第 9 轮重新量）：
      //     行 y=520  5 个按钮，x 26→226，**行宽只有 200**，右边空着 133px
      //     补到 44 是 5×44 + 4×8 = 252，落在 26→278 —— **装得下**
      //     另一行（Wrap lines / Copy）右对齐在 234→302，加宽后 222→314 —— 也装得下
      //
      // ⚠️ 走 setProperty 而非 CSS 规则的原因（实测踩到）：
      //   这些按钮的 `min-width` 本身就是**行内 style 上的 !important**
      //   （上一轮我自己写进去的），而行内 !important 在层叠里
      //   **高于任何样式表规则** —— 哪怕选择器特异性再高、哪怕也带 !important。
      //   实测：把规则插进真表里，盒子纹丝不动；只有再写一次行内才动。
      //   所以 custom.css 的 A24 只是兜底，真正让它生效的是这里。
      // ⚠️ 这里的门槛**必须是目标值 44，不能是旧值 32**。
      //   写成 `>= 32` 时有个隐蔽的死角：按钮一旦被设成 32，
      //   下一轮扫描量到 32 就不再匹配 → 永远升不上去，
      //   **卡死在 32**。实测就是这样：明明目标已经是 44，
      //   页面上的按钮却一直停在 32，而且每次扫描都「跑过了」。
      //   按目标值判定天然幂等、且能把旧版留下的 32 一起救回来。
      if (r.width >= 44) continue
      el.setAttribute('data-qx-narrow-tap', '1')
      el.style.setProperty('min-width', '44px', 'important')
      el.style.setProperty('min-height', '44px', 'important')
    }
  }

  // ══════════════════════════════════════════════════════════════════
  // 状态条 → 工具栏里的一个按钮 + 点击弹层
  // ══════════════════════════════════════════════════════════════════

  /**
   * 找出 composer 工具栏那一层横排容器 —— **按行为找，不写死层级**。
   *
   * 实测（359dp）工具栏里所有 `[data-slot="conversation.input.*"]`
   * 都是 `display:contents`，真正参与排版的是它们的父包装盒：
   *     DIV[16,531,320,108]  ← 工具栏行（我们按宽度 + 含左右两组来认它）
   *       ├ DIV[24,561,92,44]   [slot=input.left]   「+」+ 权限
   *       ├ DIV[186,561,46,44]  [slot=input.model]  模型
   *       ├ DIV[240,561,40,44]  [slot=input.activity] 麦克风
   *       └ 发送
   *   中间 116…186 有 **70px 空档** —— 状态按钮正好放这儿。
   *
   * 为什么必须「找」而不是直接 `document.querySelector` 某个 data-slot：
   *   工具栏行**没有自己的 slot**，而 left/right 都是 contents（盒模型为 0×0），
   *   直接拿它们的 parentElement 拿到的是「左组」不是「行」。
   */
  function findToolbarRow() {
    const left = doc.querySelector('[data-slot="conversation.input.left"]')
    if (!left) return null
    let node = left.parentElement
    for (let i = 0; i < 5 && node; i += 1, node = node.parentElement) {
      const cs = win.getComputedStyle(node)
      if (cs.display !== 'flex' || cs.flexDirection.indexOf('row') !== 0) continue
      const r = node.getBoundingClientRect()
      if (r.width < win.innerWidth * 0.7) continue
      if (!node.querySelector('[data-slot="conversation.input.right"]')) continue
      // 打标记，让 CSS 据此微调间距 ——
      // ⚠️ 不要试图用 CSS 选择器找这一层：`input.left` 在里面是**孙级**
      //    （行 → tools → left），写 `div:has(> [slot=input.left])` 一条都不命中。
      //    层级会变，标记不会。
      node.setAttribute('data-qx-toolbar', '1')
      return node
    }
    return null
  }

  /**
   * 从 DSH 自己的状态行里读出全部统计字段。
   *
   * ⚠️ **只读不删**：状态行里每一个字段（轮数 / 步数 / tok/s / 总 tok /
   *    缓存命中率 / 上下文占用）都要原样带进弹层。
   *    把它们收进按钮只改变**呈现位置**，不删任何信息 ——
   *    这是 P1「不改 DSH 的内容」在本轮的具体落法。
   *
   * 读不到就返回空数组，弹层会显示「暂无统计」而不是空白。
   */
  function readStats() {
    const seen = {}
    const out = []
    for (const node of findStatButtons()) {
      const parts = [...node.childNodes]
        .map((c) => (c.textContent || '').trim())
        .filter(Boolean)
      const text = (node.getAttribute('aria-label') || parts.join(' ') || node.textContent || '')
        .trim()
      if (!text) continue
      const key = text.replace(/\s+/g, ' ')
      if (seen[key]) continue
      seen[key] = 1
      out.push(key)
    }
    return out
  }

  /** 弹层：把全部统计字段列出来。 */
  function openStatusSheet() {
    for (const old of doc.querySelectorAll('[data-qx-statussheet]')) old.remove()
    const stats = readStats()
    const rows = stats.length
      ? stats.map((text) => el('div', { class: 'qx-stat-row' }, [
        el('span', { class: 'qx-stat-text', text }),
      ]))
      : [el('div', { class: 'qx-stat-row qx-stat-row--empty', text: '本会话暂无统计' })]

    const sheet = el('div', { class: 'qx-sheet qx-stat-sheet', 'data-qx-sheet': '', 'data-qx-statussheet': '' }, [
      el('div', { class: 'qx-sheet-grab' }),
      el('div', { class: 'qx-sheet-head' }, [
        el('span', { class: 'qx-sheet-title', text: '会话统计' }),
        el('button', {
          class: 'qx-sheet-x', type: 'button', text: '✕', 'aria-label': '关闭',
          onclick: () => sheet.remove(),
        }),
      ]),
      el('div', { class: 'qx-stat-list' }, rows),
    ])
    doc.body.appendChild(sheet)
    // 点背景关闭：手机上比找那个 ✕ 顺手
    sheet.addEventListener('click', (event) => {
      if (event.target === sheet) sheet.remove()
    })
  }

  /**
   * 在工具栏中间装一个状态按钮。
   *
   * 为什么按钮上只放一个数：**上下文占用**。
   * 实测它是那一堆里唯一「会随时间逼近上限」的量（79%），
   * 也是用户真正会盯着看的；轮数/步数/tok 是「跑完了才知道」的历史值，
   * 放在弹层里不影响使用。
   */
  function installStatusButton() {
    const row = findToolbarRow()
    if (!row) return false
    if (row.querySelector('.qx-status-btn')) return true

    const rightSlot = row.querySelector('[data-slot="conversation.input.right"]')
    const rightGroup = rightSlot && rightSlot.parentElement

    const btn = el('button', {
      class: 'qx-status-btn', type: 'button',
      'aria-label': '查看会话统计', title: '会话统计',
    })
    btn.appendChild(el('b', { class: 'qx-status-val', text: '—' }))
    btn.addEventListener('click', openStatusSheet)

    // ⚠️⚠️ **插在「发送」之前，不要当行的最后一个子元素。**
    //   实测：appendChild 会让它落在发送键**右边**（实测 x=287，而发送在
    //   233…281）——「发消息」旁边挂一个「上下文 79%」的读数，
    //   语义上是干扰；而放在发送键**左边**才是「辅助信息、主操作在右」
    //   （iOS / Material 的通用排法）。
    //
    //   另外 ⚠️ 这一行的 flex 分配是按「恰好两个子元素」调的：
    //   trailing 带 flex-grow，**多出第三个子元素**时它会把剩余空间全吃掉
    //   （实测 142 → 233），而 tools 可收缩（92 → **5px**，「+」被压没）。
    //   所以药丸必须待在 tools 与 trailing **之间**：
    //   实测 92 + 41 + 142 + 2×6 = 287 ≤ 304，整排稳在一行。
    const anchor = [...row.children].find((k) => k !== rightGroup
      && k.querySelector('[data-slot="conversation.input.model"]'))
    if (anchor) row.insertBefore(btn, anchor)
    else if (rightGroup && rightGroup.parentElement === row) row.insertBefore(btn, rightGroup)
    else row.appendChild(btn)

    syncStatusButton()
    return true
  }

  /**
   * 「计划编辑器」在场时给工具栏打 `data-qx-plan`，CSS 据此放开行高。
   *
   * 为什么需要它：custom.css B13 把工具栏行**写死 52px**
   * （这一行内容就是 44px + 8px padding，写死能白拿回 92px 正文高度），
   * 但计划编辑器展开时需要更多高度，否则会被切掉。
   *
   * 判据用「有尺寸」而不是「有子节点」：该槽位是 `display:contents`，
   * 即使空着也会在 DOM 里存在 —— 按子节点判会永远命中。
   */
  function syncPlanSlot() {
    const row = findToolbarRow()
    if (!row) return
    const plan = doc.querySelector('[data-slot="conversation.input.plan"]')
    let on = false
    if (plan) {
      const r = plan.getBoundingClientRect()
      const cs = win.getComputedStyle(plan)
      on = cs.display !== 'none' && cs.visibility !== 'hidden'
        && (r.height > 1 || r.width > 1)
      if (!on) {
        // contents 槽自己没盒子，就看它里面有没有可见内容
        on = [...plan.children].some((c) => {
          const cr = c.getBoundingClientRect()
          return cr.height > 1 && cr.width > 1
        })
      }
    }
    if (on) row.setAttribute('data-qx-plan', '1')
    else row.removeAttribute('data-qx-plan')
  }

  /**
   * 把上下文占用从 DSH 自己的状态行里抠出来，显示到按钮上。
   *
   * 实测那一行里有一个 `79%` 的圆环按钮，aria-label 是空的，
   * 只能靠「文本形如 n%」这个**行为**认 —— 这正是 §4.3 说的
   * 「class 不可用但结构不稳时，按行为找元素」。
   */
  function syncStatusButton() {
    const btn = doc.querySelector('.qx-status-btn')
    if (!btn) return
    let pct = null
    for (const node of findStatButtons()) {
      const t = (node.textContent || '').trim()
      // ⚠️ 只要**整段文字就是一个百分数**的那个按钮。
      //    实测踩过：宽泛地找 `/(\d+)%/` 会先撞上
      //    「89.2M tok · Cache hit **98%**」——那是**缓存命中率**，
      //    拿它当「上下文占用」显示，数字大得吓人，而且含义完全错。
      //    上下文占用那个按钮的文字**只有** `79%`（旁边是个圆环），
      //    所以「整段就是一个百分数」才是能区分二者的判据。
      const m = /^(\d{1,3})\s*%$/.exec(t)
      if (m) { pct = parseInt(m[1], 10); break }
    }
    const val = btn.querySelector('.qx-status-val')
    if (!val) return
    if (pct === null) {
      btn.setAttribute('data-qx-empty', '1')
      val.textContent = '—'
      btn.setAttribute('aria-label', '会话统计')
      return
    }
    btn.removeAttribute('data-qx-empty')
    val.textContent = pct + '%'
    btn.setAttribute('aria-label', '会话统计，上下文已用 ' + pct + '%')
    // 圆环的进度用 CSS 变量传过去（conic-gradient 读它）
    btn.style.setProperty('--qx-p', String(pct))
    // 越满越要显眼：60% 起变色，85% 起用警示色
    btn.setAttribute('data-qx-level', pct >= 85 ? 'high' : (pct >= 60 ? 'mid' : 'low'))
  }

  /**
   * 给侧栏里「工作区 / 会话」分区根打标记，交给 CSS 强制单列堆叠。
   *
   * 🔴 真机实测（359×691 与 359×789 都复现）：
   *     root        [12,238,347,335]  display:flex  **column**  wrap:wrap
   *       ├ sectionHeader [12,240,213,36]   「Workspaces」+ 搜索/筛选/添加
   *       └ listArea      [217,238,245,622]  会话列表
   *
   *   列表被推到 **x=217**（右边缘 462，视口只有 359）——
   *   屏幕上就是 `egui_hikvision_ap` / `补充命令输入文件` 这样被切掉。
   *
   * 成因：`column + wrap` 的 flex，**换行是往右边换一列**。
   * 两个子元素高度 36 + 622 = 658 > 容器 335，于是 listArea 被放到了第二列。
   * DSH 在宽侧栏上这么写没问题，在 359dp 上就把主内容推出屏幕了。
   *
   * ⚠️ 为什么用 `grid` 而不是把 `flex-direction` 改成 `row`：
   *   `row + wrap` 实测也能摆对（maxRight 241），但它是**靠内容超高触发换行**
   *   才换到下一行的 —— 视口一高（真机 789dp）就可能不换行、重新并排。
   *   「单列 grid」是**无条件**堆叠，不依赖任何测量值。
   *
   * 判据按**行为**找，不写 class：侧栏里「至少两个孩子、且至少一个孩子里装着
   * 会话行、且自己横跨大部分视口」的最内层容器。
   * 从会话行往上走，取**第一个**合格的 —— 天然就是最内层那个。
   */
  function markWorkspaceRoot() {
    const side = doc.querySelector('[data-slot="sidebar"]')
    if (!side) return 0
    let marked = 0
    for (const row of side.querySelectorAll('[data-row-key]')) {
      // ⚠️⚠️ **必须取「最外层」合格祖先，而不是第一个。**
      //   实测链条（从行往上）：
      //     i0 _root_38jqx_3       [221,…]  行
      //     i1 bhn1Oq_groupSection [221,…]  ← 第一个合格的，**改它没用**
      //     i2 bhn1Oq_list        [217,…]  第二个合格的
      //     i3 bhn1Oq_treeBody    [221,…]  ← 偏移从这里开始
      //     i4 bhn1Oq_listArea    [217,…]  ← **真正的病根**
      //   取第一个 → 标记落在 groupSection 上，偏移一点没动；
      //   截图上「看着好了」是因为那次结构层级不同，属于**碰巧**。
      //
      //   正确做法：把整条链上**所有**合格的都标上
      //   （grid 单列对中间层无害，对根层才是修复）。
      let n = row.parentElement
      for (let i = 0; i < 6 && n && side.contains(n); i += 1, n = n.parentElement) {
        let direct = 0
        for (const c of n.children) if (c.querySelector('[data-row-key]')) direct += 1
        if (direct < 1 || n.children.length < 2) continue
        const r = n.getBoundingClientRect()
        if (r.width < win.innerWidth * 0.7) continue
        if (!n.hasAttribute('data-qx-wsroot')) {
          n.setAttribute('data-qx-wsroot', '1')
          marked += 1
        }
      }
    }
    return marked
  }

  /**
   * 标记侧栏行里「那一段标题文字」。
   *
   * 🔴 A22 把行宽从 376 压到 323（回到视口内）之后暴露出来的后果：
   *     行  [12,434,323,48]  display:flex  row
   *       ├ slot [20,434, 16,20]
   *       ├ title[24,454,320,20]   ← 320 宽，**换行到了第二排**
   *       ├ time [20,474, 16,20]   ← 被挤到第三排
   *       └ ctx  [256,449, 72,17]  ← 与标题**重叠成一团**
   *     16 + 320 + 14 = 350 > 323 → 整行折成三行，48px 的行装不下。
   *     截图上表现为：长标题冲出卡片圆角、时间掉到下一行、
   *     标题和「上下文 51%」糊在一起。
   *
   * 修法是**给标题收缩能力**（`min-width:0` + 省略号）并锁死行不换行。
   *
   * ⚠️ 为什么用行为找而不是「第几个子元素」：
   *   实测子元素顺序是 slot / title / time / rowActions / 我们插的提示条，
   *   顺序会随 DSH 版本变；而「文字最多的那个非我们元素」是稳定的语义。
   *   §5.3 早就说过：写死「第几个子级是什么」会散架。
   */
  function markRowTitle() {
    const side = doc.querySelector('[data-slot="sidebar"]')
    if (!side) return 0
    let marked = 0
    for (const row of side.querySelectorAll('[data-row-key]')) {
      let best = null
      let bestLen = 0
      for (const k of row.children) {
        if (k.classList && (k.classList.contains('qx-hint-sparse')
          || k.classList.contains('qx-hint-ctx'))) continue
        const t = (k.textContent || '').trim()
        if (t.length <= bestLen) continue
        best = k
        bestLen = t.length
      }
      if (!best || bestLen < 2) continue
      if (!best.hasAttribute('data-qx-rowtitle')) marked += 1
      best.setAttribute('data-qx-rowtitle', '1')
    }
    return marked
  }

  /**
   * 标记侧栏树里的「New Session」填充药丸 → `data-qx-nspill`
   *
   * 🔴 第 20 轮：这是整个侧栏**最像移动网页**的一个元素。
   *     实测 `[role="tree"]` 内**唯一**的填充块：
   *       <DIV> «New Session»  y504  318×44
   *       底 rgba(255, 255, 255, 0.08)   圆角 12px   内距 0 8px
   *     它夹在一堆线性条目中间，截图上一眼就是「传统移动页」。
   *     VSCode 的 explorer 里**没有**这种大填充块。
   *
   * ⚠️ 为什么 CSS 选不出来（这轮踩过）：
   *     它就是一个普通的 `[role="treeitem"]`，圆角/内距/子元素
   *     和文件夹行、会话行**完全同构**（SPAN 16×20 + SPAN 文本）。
   *     唯一的区别是**文字是「New Session」**——CSS 无法匹配文本。
   *     试过按结构猜（有 svg = 文件夹 / 有 0×0 div = 会话），
   *     结果 New Session 行**两者都不满足**，但还有别的行也落入空集。
   *     ⇒ 按项目既有约定，**用 JS 打标记**（同 `data-qx-wsroot` /
   *       `data-qx-rowtitle`），CSS 只认 `[data-qx-nspill]`。
   *
   * 只**标记**，不移动、不删除、不改结构（AGENTS 契约：定制层不做 DOM 增删移动）。
   */
  function markNewSessionPill() {
    let marked = 0
    const trees = doc.querySelectorAll('[role="tree"]')
    for (const tree of trees) {
      for (const item of tree.querySelectorAll('[role="treeitem"]')) {
        const t = (item.textContent || '').replace(/\s+/g, ' ').trim()
        // 精确匹配，避免把标题里含 "new session" 的真实会话误伤
        if (t !== 'New Session' && t !== '新建会话') continue
        if (!item.hasAttribute('data-qx-nspill')) {
          item.setAttribute('data-qx-nspill', '1')
          marked += 1
        }
      }
    }
    return marked
  }

  /**
   * 标记侧栏根的 5 个直接子块（第 20 轮 B18 换皮用）。
   *
   * 🔴 为什么不能再用 CSS 选择器考古（这轮已经栽了三次）：
   *     ① `[data-slot="sidebar"]`   → 命中 0，**侧栏根根本没有 data-slot**
   *     ② `div[aria-label="New session"]` → 命中 0
   *        顶部那个大药丸其实是 **`<button>`** 不是 div
   *     ③ `button[aria-label="New session"]` → **命中 2**
   *        274×44（logo 区的「新会话」入口）**和** 326×44（顶部大药丸）
   *        —— 同一个 aria-label 两个元素，靠选择器分不开
   *
   * ⇒ 一次把 5 块全标好，CSS 只认 `data-qx-sb-*`，不再猜。
   *
   * 块位（自上而下，实测 359×691dp）：
   *   sb-logo     60   含 Collapse sidebar 图标按钮
   *   sb-primary  44   顶部大药丸「New Session」326×44 圆角12
   *   sb-panels  100   Plugins / Automation tasks 两个 48 按钮
   *   sb-tree    367   Workspaces + 树（flex:1 1 0%）
   *   sb-footer   96   Mobile access / Settings
   */
  function markSidebarBlocks() {
    let marked = 0
    const 树 = doc.querySelector('[role="tree"]')
    if (!树 || !树.getBoundingClientRect().width) return 0
    // 往上走到侧栏根：从树起第 4 层是根（实测 L0..L6）
    let 根 = 树
    for (let i = 0; i < 6 && 根.parentElement; i += 1) 根 = 根.parentElement
    for (const 块 of 根.children) {
      if (块.getBoundingClientRect().width <= 0) continue
      // ⚠️ 必须 **自身或后代** 都查 —— 顶部那个大药丸**本身就是**
        //    `<button aria-label="New session">`，不是「装着 button 的容器」。
        //    只用 querySelector（只搜后代）会把块1 整块漏掉，
        //    症状是 `data-qx-sb-primary` MISSING，而其它 4 块都正常。
      const 有 = sel => 块.matches(sel) || !!块.querySelector(sel)
      let 名 = ''
      if (有('[role="tree"]')) 名 = 'tree'
      else if (有('[data-slot="sidebar.settings"]')) 名 = 'footer'
      else if (有('button[aria-label="Collapse sidebar"]')) 名 = 'logo'
      else if (有('button[aria-label="Plugins"], button[aria-label="Automation tasks"]')) 名 = 'panels'
      else if (有('button[aria-label="New session"]')) 名 = 'primary'
      if (!名) continue
      for (const 旧 of ['logo', 'primary', 'panels', 'tree', 'footer']) {
        块.removeAttribute('data-qx-sb-' + 旧)
      }
      if (!块.hasAttribute('data-qx-sb-' + 名)) marked += 1
      块.setAttribute('data-qx-sb-' + 名, '1')
    }
    return marked
  }

  /**
   * 摘掉侧栏上 DSH 自己写的内联 `min-height: 44px !important`（B18 换皮的必要前提）。
   *
   * 🔴🔴 本项目最贵的一条 CSS 教训（第 20 轮实测踩穿）：
   *
   *   `hHd-Xa_newSession`（顶部大药丸）与 `hHd-Xa_panelRow`（Plugins /
   *   Automation tasks）上带着**内联** `min-height: 44px !important`。
   *   它**不是 custom.js 写的**（全项目 4 处 `min-height` 内联都在
   *   会话区 / 点触点修复 / 细图按钮里，够不到侧栏），**是 DSH 运行时自己写的**。
   *
   *   ⇒ CSS **永远压不过它**。实测阶梯：
   *       `[data-qx-sb-primary]{min-height:33px!important}`            → 44
   *       `[data-qx-sb-primary][data-qx-sb-primary]{…!important}`      → 44
   *       再叠 `button[...]` / 三重属性 / 放到最后                        → **还是 44**
   *       只有 `el.style.setProperty('min-height','40px','important')`  → 40 ✅
   *
   *   原因：**内联 `!important` 排在作者样式表 `!important` 之上**，
   *   且这一层**与选择器特异性无关** —— 叠多少遍 `[x][x]` 都一样。
   *
   *   ⚠️ 反过来也解释了第 19 轮的一个**假成功**：
   *   B16「把侧栏底栏压到 48」当时真的生效了 —— 但那是因为 **48
   *   恰好等于内联值**，不是 B16 压过了谁。现在改成 40 立刻打回原形。
   *   ⇒ **别把「值恰好相同」当成「规则生效了」。**
   *
   * 做法：标记时把这条内联摘掉，尺寸改由 CSS 的 `data-qx-sb-*` 规则接管。
   * 清理函数里 `marks` 已包含 sb-*，会再 `removeProperty` 一次，幂等。
   */
  function relaxSidebarInlineHeights() {
    let n = 0
    const 目标 = []
    const p = doc.querySelector('[data-qx-sb-primary]')
    if (p) 目标.push(p, ...p.querySelectorAll('*'))
    for (const k of ['panels', 'footer', 'logo']) {
      const b = doc.querySelector('[data-qx-sb-' + k + ']')
      if (b) 目标.push(b, ...b.querySelectorAll('*'))
    }
    for (const el of 目标) {
      const v = el.style && el.style.getPropertyPriority
        && el.style.getPropertyValue('min-height')
      if (!v) continue
      if (el.style.getPropertyPriority('min-height') !== 'important') continue
      el.style.removeProperty('min-height')
      n += 1
    }
    return n
  }

  /**
   * 工作区页打开时把 ☰ 藏掉。
   *
   * ⚠️ 真机实测（359dp）：侧栏展开后 DSH 会把对话顶栏换成
   *    `headerBlank`，我们放在 leading 空槽里的 ☰ 被顶到 **x=381** ——
   *    视口只有 359，**整整 66px 在屏幕外**，一次审查报出 5 个 high 级溢出。
   *
   * 而且它那时**本来就没用**：工作区页自带收起键（顶栏右侧那个 ▣）。
   * 所以直接藏掉是双赢 —— 少一次误触，也不再溢出。
   *
   * 判据用 `data-sidebar-collapsed` 的有无（展开时该属性被移除），
   * 与 CSS A10 是同一套语义。
   */
  function syncMenuButton() {
    const btn = doc.querySelector('.qx-menu-btn')
    if (!btn) return
    const frame = doc.querySelector('[data-slot="root"] > *')
    const expanded = frame ? !frame.hasAttribute('data-sidebar-collapsed') : false
    btn.style.setProperty('display', expanded ? 'none' : '', 'important')
  }

  // ══════════════════════════════════════════════════════════════════
  // 窄屏导航：☰ → 工作区独立页
  // ══════════════════════════════════════════════════════════════════

  /**
   * 在顶栏左侧那个**空的** leading 槽里放一个菜单按钮。
   *
   * 落点实测：`[data-slot="conversation.header.leading"]`
   *   display: contents、innerHTML 为空 ""、位置是顶栏最左边那个 0×48 的空位。
   * 所以这是 DSH 自己留出来的槽 —— 我们填进去既不用挪它的元素，
   * 也不用新开一个会盖住顶栏的浮层。
   *
   * ⚠️ 只在窄屏插：宽屏下 DSH 自己的 rail 就在那儿，再加一个就是两套导航。
   */
  function installMenuButton() {
    const NARROW = 760
    const slot = doc.querySelector('[data-slot="conversation.header.leading"]')
    if (!slot) return false
    // 已经有东西了就不插 —— 宁可少一个按钮，也不要叠两个
    if (slot.querySelector('.qx-menu-btn')) return true
    if (slot.children.length > 0) return false

    const btn = el('button', {
      class: 'qx-menu-btn',
      type: 'button',
      'aria-label': '打开工作区',
    })
    btn.appendChild(el('i'))          // 图标本体，纯 CSS 画，见 custom.css A12
    btn.addEventListener('click', () => { openWorkspacePage() })
    slot.appendChild(btn)

    // 视口从窄变宽（比如折叠屏展开 / 横屏）时把按钮撤掉，交还给 rail
    //
    // ⚠️ **重新查一次 slot，不能用闭包里那个 `slot`**：
    //   DSH 换会话 / 重渲染顶栏时会整块替换掉 leading 槽，
    //   闭包里存的是**已经被换掉的旧节点**。往它上面 append 等于
    //   把按钮挂在一个游离的树上 —— 界面上看不到，但 `querySelector`
    //   还查得到，于是看起来「按钮还在，就是不出现」。
    const watch = () => {
      if (win.innerWidth > NARROW) {
        if (btn.parentNode) btn.remove()
        return
      }
      if (btn.parentNode) return
      const live = doc.querySelector('[data-slot="conversation.header.leading"]')
      if (live && live.children.length === 0) live.appendChild(btn)
    }
    win.addEventListener('resize', watch)
    state.onResize = watch
    return true
  }

  /**
   * 打开工作区页。
   *
   * ⚠️ 窄屏下 rail 是 `display:none`（custom.css A11），
   *    但 **`display:none` 元素上的 `.click()` 照样派发** —— 所以直接点
   *    DSH 自己的「Open sidebar」按钮就行。
   *
   * 为什么不自己增删 `data-sidebar-collapsed`：
   *   那是 DSH 内部状态机的输出，手改会和它打架（展开动画、localStorage
   *   里的 dsh.workspace.view.v5 都会对不上）。
   */
  function openWorkspacePage() {
    const btn = findSidebarToggle(true)
    if (btn) {
      btn.click()
      return true
    }
    return false
  }

  // ══════════════════════════════════════════════════════════════════
  // 启动
  // ══════════════════════════════════════════════════════════════════

  try {
    await refreshSessions()
  } catch (error) {
    state.lastError = String(error)
  }

  // ── 第 11 轮：先扫掉上一轮遗留的层，再插自己的 ────────────────────
  //
  // 实测在热重载多次后页面上**同时存在 2 个 .qx-layer**。
  // dispose 明明有 `layer.remove()`（第 1500 行），所以只能是
  // **两个 mount 同时活着** —— 旧实例还没被 dispose，新实例已经建好了层。
  //
  // 眼下它们 `pointer-events:none`、没有子元素，谁也挡不住，
  // 所以肉眼看不出问题 —— 但这是一个**只会累积、不会自愈**的泄漏，
  // 而且闸门原来只断言「已插入、zIndex 正确」，**没断言只有一个**。
  //
  // 「断言了存在」≠「断言了存在且唯一」—— 这是本轮补的第二条同类教训。
  //
  // 只清自己的层（按 class），不碰任何 DSH 节点：
  // 按项目约定，定制层不插入/移动 DSH 的结构，这里清理的是自己的产物。
  for (const stale of doc.querySelectorAll('.qx-layer')) stale.remove()

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
    markClippedRows()
    // ⚠️ 必须紧跟在 markClippedRows 之后 —— 它认的是药丸身上
    //    markClippedRows 刚打上的 [data-qx-clipped]，顺序反了就一个都标不上。
    markStatusRow()
    markTinyText()
    markShortTapRows()
    syncMenuButton()
    // ⚠️ 必须每轮重试：installMenuButton 只在启动时调一次，而顶栏是**异步**渲染的
    //    —— 实测整页重载后 `[data-slot="conversation.header.leading"]` 还没出现，
    //    那一行就返回 false 并**永远不再重试**，☰ 于是彻底消失。
    //    症状是「导航入口没了」，但界面看不出任何报错。
    installMenuButton()
    // 同理：工具栏也是异步渲染的，状态按钮必须每轮重试 + 同步数值
    installStatusButton()
    syncStatusButton()
    syncPlanSlot()
    markWorkspaceRoot()
    markRowTitle()
    markNewSessionPill()
    markSidebarBlocks()
    relaxSidebarInlineHeights()
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
  installMenuButton()

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
    if (state.onResize) win.removeEventListener('resize', state.onResize)
    const menuBtn = doc.querySelector('.qx-menu-btn')
    if (menuBtn) menuBtn.remove()
    // 状态按钮插在 DSH 自己的工具栏行里（不在 layer 里），必须单独清
    const statusBtn = doc.querySelector('.qx-status-btn')
    if (statusBtn) statusBtn.remove()
    const toolbars = doc.querySelectorAll('[data-qx-toolbar]')
    for (let i = 0; i < toolbars.length; i += 1) toolbars[i].removeAttribute('data-qx-toolbar')
    // 提示条插在 DSH 自己的行里，不在 layer 里 —— 必须单独清，
    // 否则热替换之后旧的还留着，新 mount 会再插一遍（出现过两个「上下文 79%」）
    const leftovers = doc.querySelectorAll('.qx-hint-sparse, .qx-hint-ctx, .qx-full-btn')
    for (let i = 0; i < leftovers.length; i += 1) leftovers[i].remove()
    const pendingRows = doc.querySelectorAll('.qx-row--pending')
    for (let i = 0; i < pendingRows.length; i += 1) pendingRows[i].classList.remove('qx-row--pending')
    const marks = doc.querySelectorAll('[data-qx-clipped], [data-qx-tiny], [data-qx-short-tap], [data-qx-inline-tap], [data-qx-narrow-tap], [data-qx-status], [data-qx-thin-media], [data-qx-plan], [data-qx-toolbar], [data-qx-wsroot], [data-qx-rowtitle], [data-qx-nspill], [data-qx-sb-logo], [data-qx-sb-primary], [data-qx-sb-panels], [data-qx-sb-tree], [data-qx-sb-footer]')
    for (let i = 0; i < marks.length; i += 1) {
      marks[i].removeAttribute('data-qx-clipped')
      marks[i].removeAttribute('data-qx-tiny')
      marks[i].removeAttribute('data-qx-short-tap')
      marks[i].removeAttribute('data-qx-inline-tap')
      marks[i].removeAttribute('data-qx-narrow-tap')
      marks[i].removeAttribute('data-qx-status')
      marks[i].removeAttribute('data-qx-thin-media')
      marks[i].removeAttribute('data-qx-wsroot')
      marks[i].removeAttribute('data-qx-rowtitle')
      marks[i].removeAttribute('data-qx-nspill')
      marks[i].removeAttribute('data-qx-sb-logo')
      marks[i].removeAttribute('data-qx-sb-primary')
      marks[i].removeAttribute('data-qx-sb-panels')
      marks[i].removeAttribute('data-qx-sb-tree')
      marks[i].removeAttribute('data-qx-sb-footer')
      marks[i].style.removeProperty('padding-top')
      marks[i].style.removeProperty('padding-bottom')
      marks[i].style.removeProperty('font-size')
      marks[i].style.removeProperty('min-height')
      marks[i].style.removeProperty('min-width')
    }
    if (layer) { layer.remove(); layer = null }
    for (const node of doc.querySelectorAll('[data-qx-sheet]')) node.remove()
  }
})
