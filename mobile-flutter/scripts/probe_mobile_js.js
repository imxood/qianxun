/**
 * L3 通道探针 —— 方案 B 的第一步，验「mobile.js 里的 request 真的已认证吗」
 *
 * 为什么必须先做这一步
 * --------------------
 * 整个方案 B 建立在一条假设上：
 *
 *   window.dshMobile.register(({ request }) => …) 里的 `request`
 *   是一个**已认证**的 fetch，能直接调 POST /api/session/list。
 *
 * 我在 `08-l3-contract.md` 里从源码确认了它由 `mobileRequest` 传入
 * （dsh-mobile/src/client.ts:2367），但**源码说的是「它是什么」，
 * 不是「它在你那台手机上能不能用」**。中间隔着：TLS 固定、设备凭据、
 * 配对态、Origin 校验。
 *
 * 假设错了，后面 6 步全是空中楼阁 —— 我们会基于一个拿不到数据的通道
 * 写完整个界面，最后发现全是空列表。所以这一步先跑通。
 *
 * 怎么用
 * ------
 * 1. 电脑装好 dsh-mobile 插件，跑 `dsh plugin --profile web exec dsh-mobile setup`
 * 2. 把本文件内容粘到 $DSH_HOME/mobile-access/mobile.js
 *    （或先备份原文件；恢复默认 = 删掉 mobile.js 与 mobile.css）
 * 3. 手机连上，打开 App，进 DSH 页
 * 4. 看手机上的「探针结果」浮层，或电脑浏览器 DevTools 的 Console
 *
 * 它做什么
 * --------
 * 依次验四件事，每件都**独立报告**，一项失败不阻断后面的：
 *   1. 通道存在      —— window.dshMobile 与 register 是否可用
 *   2. 已认证        —— session/list 返回 ok:true（不是 401）
 *   3. 数据完整      —— 条数、running 分布、contextPressure 占比
 *   4. 字段形状      —— projections.values 的键数与关键字段
 *
 * ⚠️ 不发任何写操作。全部是 GET 语义的 list 调用。
 */

window.dshMobile.register(async ({ request, document, window }) => {
  const lines = []
  const log = (ok, text) => { lines.push({ ok, text }) }
  const json = (v) => { try { return JSON.stringify(v) } catch { return String(v) } }

  // ── 1. 通道存在 ────────────────────────────────────────────────
  const hasChannel = typeof request === 'function'
  log(hasChannel, hasChannel
    ? '通道存在：request 是函数'
    : '通道缺失：request 不是函数 —— 方案 B 在此不成立')

  if (!hasChannel) {
    return render(lines)
  }

  // ── 2. 已认证 ─────────────────────────────────────────────────
  let body = null
  try {
    const res = await request('/api/session/list', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        type: 'client-request',
        rpcId: 'qx-probe',
        method: 'session/list',
        // ⚠️ session/list 的参数键恰好叫 _request，不是 request
        payload: { args: { _request: {} } },
      }),
    })
    log(res.ok, `HTTP ${res.status} ${res.statusText}`)
    body = await res.json()
  } catch (error) {
    log(false, `请求抛错：${error && error.message ? error.message : String(error)}`)
    return render(lines)
  }

  // 信封是三层：{ type, rpcId, result: { ok, value | error } }
  const result = body && body.result
  const ok = result && result.ok === true
  log(Boolean(ok), ok
    ? '已认证：result.ok === true'
    : `未通过：${json(body).slice(0, 220)}`)

  if (!ok) return render(lines)

  // ── 3. 数据完整 ───────────────────────────────────────────────
  const items = (result.value && result.value.items) || []
  log(items.length > 0, `条数：${items.length}`)

  if (items.length > 0) {
    const running = items.filter((i) => i && i.running === true).length
    log(true, `running=true：${running} / ${items.length}`)

    // agentAvailable 在本部署 392/393 为 false —— 不能当状态判据。
    // 这里只报告分布，作为「与探针一致」的交叉验证。
    const avail = items.reduce((acc, i) => {
      const k = String(i && i.agentAvailable)
      acc[k] = (acc[k] || 0) + 1
      return acc
    }, {})
    log(true, `agentAvailable：${json(avail)}`)

    const withCtx = items.filter((i) => {
      const v = (i && i.projections && i.projections.values) || {}
      return v.contextPressure && v.contextPressure.contextWindow
    })
    log(true, `带 contextPressure：${withCtx.length} (${(withCtx.length / items.length * 100).toFixed(0)}%)`)

    if (withCtx.length > 0) {
      const pcts = withCtx
        .map((i) => i.projections.values.contextPressure.projectedTokens
          / i.projections.values.contextPressure.contextWindow * 100)
        .sort((a, b) => a - b)
      const at = (p) => pcts[Math.min(pcts.length - 1, Math.floor(pcts.length * p))]
      log(true, `上下文 p50 ${at(0.5).toFixed(1)}% · p90 ${at(0.9).toFixed(1)}% · max ${pcts[pcts.length - 1].toFixed(1)}%`)
      log(true, `>50%: ${pcts.filter((p) => p > 50).length} 条 · >65%: ${pcts.filter((p) => p > 65).length} 条`)
    }

    const qActive = items.filter((i) => {
      const v = (i && i.projections && i.projections.values) || {}
      return v.userQuestions && v.userQuestions.active && v.userQuestions.active.length > 0
    }).length
    log(true, `userQuestions.active 非空：${qActive} 条`)
  }

  // ── 4. 字段形状 ───────────────────────────────────────────────
  const first = items.find((i) => i && i.projections && i.projections.values)
  if (first) {
    const keys = Object.keys(first.projections.values).sort()
    log(true, `projections.values 字段数：${keys.length}`)
    log(true, `字段：${keys.join(', ')}`)
    // 与 05 §0.2.3 记的 20 个对照。少一个就说明这一版协议变了，
    // 05 的声明要跟着改 —— 不要只改代码注释。
    log(keys.length === 20, keys.length === 20
      ? '字段数 20，与 05 §0.2.3 一致'
      : `字段数 ${keys.length} ≠ 20，与 05 §0.2.3 不一致 —— 需核对并更新文档`)

    const v = first.projections.values
    const has = (k) => Object.prototype.hasOwnProperty.call(v, k)
    log(has('contextPressure') && !!v.contextPressure.contextWindow,
      'contextPressure.contextWindow 存在（上下文百分比的唯一分母）')
  } else {
    log(false, '没有任何带 projections.values 的条目')
  }

  return render(lines)

  // ── 结果浮层 ───────────────────────────────────────────────────
  function render(entries) {
    const pass = entries.filter((e) => e.ok).length
    const fail = entries.filter((e) => !e.ok).length

    const box = document.createElement('div')
    box.style.cssText = [
      'position:fixed', 'z-index:2147483647', 'left:8px', 'right:8px',
      'top:calc(env(safe-area-inset-top) + 8px)',
      'max-height:70vh', 'overflow:auto', 'z-index:99999',
      'background:#0b0e13', 'color:#e8edf2',
      'border:1px solid rgba(255,255,255,.16)', 'border-radius:12px',
      'padding:12px 14px', 'font:12px/1.6 ui-monospace,Menlo,monospace',
      'box-shadow:0 12px 40px rgba(0,0,0,.5)', 'pointer-events:auto',
    ].join(';')

    const head = document.createElement('div')
    head.style.cssText = 'font-weight:700;font-size:13px;margin-bottom:8px;color:' + (fail === 0 ? '#3DDC97' : '#FFC24B')
    head.textContent = `L3 通道探针 · 通过 ${pass} · 失败 ${fail}`
    box.append(head)

    for (const entry of entries) {
      const row = document.createElement('div')
      row.style.cssText = 'display:flex;gap:6px;padding:3px 0;border-top:1px solid rgba(255,255,255,.07);word-break:break-all'
      const mark = document.createElement('span')
      mark.style.cssText = 'flex:0 0 auto;font-weight:700;color:' + (entry.ok ? '#3DDC97' : '#FF7B72')
      mark.textContent = entry.ok ? '✓' : '✗'
      const text = document.createElement('span')
      text.textContent = entry.text
      row.append(mark, text)
      box.append(row)
    }

    const close = document.createElement('button')
    close.textContent = '关闭'
    close.style.cssText = 'margin-top:10px;width:100%;padding:9px;border-radius:8px;border:1px solid rgba(255,255,255,.2);background:#1a2029;color:#e8edf2;font:600 13px system-ui'
    close.onclick = () => box.remove()
    box.append(close)

    document.body.append(box)
    // 同时打一份到 console，电脑端调试更方便
    console.log('[qx-probe]', lines)
    window.setTimeout(() => box.remove(), 60000)
  }
})
