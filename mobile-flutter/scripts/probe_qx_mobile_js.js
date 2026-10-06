/**
 * 千寻自有网关的 L3 通道探针（qianxun 自己的 /qx-mobile 定制层）。
 *
 * 与 dsh-mobile 的 `dshMobile` 有三处不同，别混用：
 *   入口   window.qxMobile.register（不是 dshMobile）
 *   上下文 只有 { root, document, window }（**没有 request**）
 *   文件   mobile-access/custom.js + custom.css（不是 mobile.js / mobile.css）
 *
 * 「没有 request」不是问题：脚本跑在同源页面里，`window.fetch` 本身就
 * 带 cookie。dsh-mobile 之所以传 request，是它额外做了 CSRF 头注入
 * （gateway.ts:129 改写 window.fetch）。千寻的网关用 qx_token cookie，
 * 走的是 DSH 自己那套 CSRF —— 与 Web 端相同，所以不需要额外包装。
 *
 * 本探针验四件事：
 *   1. qxMobile 入口存在（千寻是否已注入 bootstrap）
 *   2. 已认证（带 cookie 调 session/list 是否 ok:true）
 *   3. 数据完整（条数、字段签名，应与网关实测一致）
 *   4. 无 Origin 头时的行为（决定壳层要不要手动补 Origin）
 *
 * 放到 <DSH_HOME>/mobile-access/custom.js
 */
window.qxMobile.register(async ({ document, window }) => {
  const lines = []
  const log = (ok, text) => { lines.push({ ok, text }) }
  const json = (v) => { try { return JSON.stringify(v) } catch { return String(v) } }

  log(true, 'qxMobile.register 已执行 —— 定制层挂载成功')

  if (typeof window.fetch !== 'function') {
    log(false, '没有 window.fetch —— 页面上下文异常')
    return render()
  }

  // ── 2. 已认证：同源 fetch 调 session/list ──────────────────────
  let body = null
  try {
    const res = await window.fetch('/api/session/list', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      // 刻意不写 credentials：同源 fetch 默认 same-origin，与 Web 端一致。
      // 若这里失败，说明网关要求显式带凭据 —— 那是壳层要知道的。
      body: JSON.stringify({
        type: 'client-request',
        rpcId: 'qx-l3-probe',
        method: 'session/list',
        payload: { args: { _request: {} } },   // ⚠️ session/list 用 _request
      }),
    })
    log(res.ok, `HTTP ${res.status} ${res.statusText}`)
    body = await res.json()
  } catch (error) {
    log(false, `fetch 抛错：${error && error.message ? error.message : String(error)}`)
    return render()
  }

  const result = body && body.result
  const ok = result && result.ok === true
  log(Boolean(ok), ok
    ? '已认证：result.ok === true'
    : `未通过：${json(body).slice(0, 240)}`)

  if (!ok) return render()

  // ── 3. 数据完整 ───────────────────────────────────────────────
  const items = (result.value && result.value.items) || []
  log(items.length > 0, `条数：${items.length}`)

  if (items.length > 0) {
    const running = items.filter((i) => i && i.running === true).length
    log(true, `running=true：${running} / ${items.length}`)

    // 字段签名：应与 probe_projection_signatures.py 的 {1,18,19,20} 一致
    const sigs = {}
    let withValues = 0
    for (const item of items) {
      const v = item && item.projections && item.projections.values
      if (v && typeof v === 'object' && !Array.isArray(v)) {
        withValues += 1
        sigs[Object.keys(v).length] = (sigs[Object.keys(v).length] || 0) + 1
      }
    }
    log(true, `带 projections.values：${withValues}`)
    log(true, `字段签名分布：${json(sigs)}`)
    const keys = Object.keys(sigs).map(Number).sort((a, b) => a - b)
    log(keys.length > 0 && keys[0] === 1,
      keys[0] === 1
        ? '签名含 1 —— 与全量实测一致（52.8% 只有 title）'
        : `⚠️ 最少字段数是 ${keys[0]}，与实测的 1 不符 —— 协议可能变了`)

    const withCtx = items.filter((i) => {
      const v = (i && i.projections && i.projections.values) || {}
      return v.contextPressure && v.contextPressure.contextWindow
    }).length
    log(true, `带 contextPressure：${withCtx}（实测应约 47%）`)

    const ids = new Set(items.map((i) => i.sessionId))
    const detached = items.filter(
      (i) => i.origin === 'subagent' && !ids.has(i.parentSessionId),
    ).length
    log(detached === 0,
      `已断开（子会话∧父不在列表）：${detached}（实测应为 0）`)
  }

  // ── 4. 裸 fetch（无自定义头）能不能过 CSRF ──────────────────────
  // 若需要额外头，custom.js 里每次请求都得自己补 —— 那是壳层与页面的
  // 一个契约差异，必须知道。
  try {
    const res = await window.fetch('/qx-mobile/version')
    log(res.ok, `无凭据 GET /qx-mobile/version → ${res.status}（401 = 该路由要鉴权，正常）`)
  } catch {
    log(false, '/qx-mobile/version 请求异常')
  }

  return render()

  function render() {
    const pass = lines.filter((e) => e.ok).length
    const fail = lines.filter((e) => !e.ok).length

    const box = document.createElement('div')
    box.style.cssText = [
      'position:fixed', 'z-index:2147483647', 'left:8px', 'right:8px',
      'top:calc(env(safe-area-inset-top) + 8px)', 'max-height:70vh', 'overflow:auto',
      'background:#0b0e13', 'color:#e8edf2',
      'border:1px solid rgba(255,255,255,.16)', 'border-radius:12px',
      'padding:12px 14px', 'font:12px/1.6 ui-monospace,Menlo,monospace',
      'box-shadow:0 12px 40px rgba(0,0,0,.5)', 'pointer-events:auto',
    ].join(';')

    const head = document.createElement('div')
    head.style.cssText = 'font-weight:700;font-size:13px;margin-bottom:8px;color:'
      + (fail === 0 ? '#3DDC97' : '#FFC24B')
    head.textContent = `千寻 L3 通道探针 · 通过 ${pass} · 失败 ${fail}`
    box.append(head)

    for (const entry of lines) {
      const row = document.createElement('div')
      row.style.cssText = 'display:flex;gap:6px;padding:3px 0;border-top:1px solid rgba(255,255,255,.07);word-break:break-all'
      const mark = document.createElement('span')
      mark.style.cssText = 'flex:0 0 auto;font-weight:700;color:'
        + (entry.ok ? '#3DDC97' : '#FF7B72')
      mark.textContent = entry.ok ? '✓' : '✗'
      const text = document.createElement('span')
      text.textContent = entry.text
      row.append(mark, text)
      box.append(row)
    }

    const close = document.createElement('button')
    close.textContent = '关闭'
    close.style.cssText = 'margin-top:10px;width:100%;padding:9px;border-radius:8px;'
      + 'border:1px solid rgba(255,255,255,.2);background:#1a2029;color:#e8edf2;font:600 13px system-ui'
    close.onclick = () => box.remove()
    box.append(close)

    document.body.append(box)
    window.console.log('[qx-l3-probe]', lines)
    window.setTimeout(() => box.remove(), 60000)
  }
})
