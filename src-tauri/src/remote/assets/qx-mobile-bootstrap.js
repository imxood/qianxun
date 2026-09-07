/* 千寻移动定制层引导 —— 由网关注入到每个 DSH 页面（</head> 前）。
 *
 * 职责（借鉴 dsh-mobile 的「文件即 UI」+ 热刷新策略）：
 * 1. 定义 window.qxMobile.register(...) —— 电脑端定制脚本 mobile-access/custom.js 的唯一入口；
 * 2. 轮询 /qx-mobile/version 的内容 hash：页面可见 5s / 隐藏 30s；
 * 3. hash 变化 → 热替换样式与脚本：样式换 <link> href（URL 带版本参数，浏览器按 URL 缓存）；
 *    脚本先挂新 root、执行成功后再卸旧 root（失败保留旧版，只报 console）；
 * 4. 网关暂不可达时静默重试，绝不影响 DSH 本体页面。
 */
(() => {
  'use strict'
  if (window.__QX_MOBILE_BOOT__) return
  window.__QX_MOBILE_BOOT__ = true

  const VERSION_URL = '/qx-mobile/version'
  const VISIBLE_MS = 5000
  const HIDDEN_MS = 30000
  const FETCH_TIMEOUT_MS = 8000

  let current = { css: '', js: '' }
  let link = null
  let root = null
  let dispose = null
  const queue = []
  let timer = 0

  window.qxMobile = Object.freeze({
    apiVersion: 1,
    /** 定制脚本唯一入口：window.qxMobile.register(({ root }) => { ...; return 清理函数 }) */
    register(mount) {
      queue.push(mount)
      flush()
    },
  })

  function ensureRoot() {
    if (root && root.isConnected) return root
    root = document.createElement('div')
    root.dataset.qxMobileRoot = ''
    document.body.appendChild(root)
    return root
  }

  function flush() {
    while (queue.length > 0) {
      const mount = queue.shift()
      try {
        const result = mount({ root: ensureRoot(), document, window })
        dispose = typeof result === 'function' ? result : null
      } catch (error) {
        console.error('[qx-mobile] 定制脚本挂载失败：', error)
      }
    }
  }

  function ensureLink() {
    if (link && link.isConnected) return link
    link = document.createElement('link')
    link.rel = 'stylesheet'
    link.dataset.qxMobileCustom = 'css'
    document.head.appendChild(link)
    return link
  }

  function fetchWithTimeout(url) {
    const controller = new AbortController()
    const timeout = setTimeout(() => controller.abort(), FETCH_TIMEOUT_MS)
    return fetch(url, { cache: 'no-store', credentials: 'same-origin', signal: controller.signal }).finally(() => {
      clearTimeout(timeout)
    })
  }

  function applyCss(hash) {
    ensureLink().href = '/qx-mobile/custom.css?v=' + hash
  }

  /** 先挂新 root 执行新脚本，成功后卸旧；失败保留旧版。 */
  async function applyJs(hash) {
    const response = await fetchWithTimeout('/qx-mobile/custom.js?v=' + hash)
    if (!response.ok) throw new Error('HTTP ' + response.status)
    const script = await response.text()

    const nextRoot = document.createElement('div')
    nextRoot.dataset.qxMobileRoot = ''
    document.body.appendChild(nextRoot)
    const previousRoot = root
    const previousDispose = dispose
    root = nextRoot
    dispose = null
    try {
      const node = document.createElement('script')
      node.textContent = script + '\n//# sourceURL=qx-mobile-custom.js'
      document.head.appendChild(node)
      node.remove()
    } catch (error) {
      nextRoot.remove()
      root = previousRoot
      dispose = previousDispose
      console.error('[qx-mobile] 定制脚本执行失败，保留旧版：', error)
      return
    }
    if (typeof previousDispose === 'function') {
      try {
        previousDispose()
      } catch (error) {
        console.warn('[qx-mobile] 旧脚本清理失败：', error)
      }
    }
    previousRoot?.remove()
    current.js = hash
  }

  async function tick() {
    try {
      const next = await fetchWithTimeout(VERSION_URL).then(response => {
        if (!response.ok) throw new Error('HTTP ' + response.status)
        return response.json()
      })
      if (next && typeof next.css === 'string' && next.css !== current.css) {
        applyCss(next.css)
        current.css = next.css
      }
      if (next && typeof next.js === 'string' && next.js !== current.js) await applyJs(next.js)
    } catch {
      /* 网关暂不可达或响应异常：保留现状，下个周期重试。 */
    }
    schedule()
  }

  function schedule() {
    clearTimeout(timer)
    timer = setTimeout(tick, document.hidden ? HIDDEN_MS : VISIBLE_MS)
  }

  document.addEventListener('visibilitychange', () => {
    if (!document.hidden) {
      clearTimeout(timer)
      tick()
    }
  })
  window.addEventListener('pageshow', event => {
    if (event.persisted) {
      clearTimeout(timer)
      tick()
    }
  })

  schedule()
})()
