/*
 * 千寻桌面外壳层 —— 外链拦截脚本（回环网关注入，仅注入千寻外壳承载的 DSH iframe）。
 *
 * 背景：WebView2 里 wry 未注册 NewWindowRequested 回调时静默吞掉一切弹窗请求，
 * DSH 聊天里 target="_blank" 的链接点了毫无反应；跨站 iframe 又够不到 Tauri IPC。
 * 本脚本把外链点击 postMessage 给外壳（qianxun 前端），由外壳分流：
 * 内置浏览器窗口（window_open_external）或系统浏览器（opener 插件）。
 *
 * 安全约定（与 src/lib/external-links.ts 对齐）：
 * - 与外壳双向握手（ready/arm）后才接管点击；直接用浏览器打开网关地址时
 *   永远等不到 arm，页面保持完全原生行为，零破坏。
 * - 所有 postMessage 只投递给外壳 origin 白名单，父页不是千寻外壳时静默丢弃，
 *   恶意页面无法借 iframe 收割用户点击的外链。
 * - 只接管 http/https 且跨源（非 DSH 站内）的链接；站内导航、mailto 等原样放行。
 */
(function () {
  'use strict';
  if (window.__qxShellLinks) return;
  window.__qxShellLinks = true;

  var TAG = '__qxShell';
  // 可信父页 origin（千寻外壳）：release tauri.localhost / dev vite 5190。
  var PARENT_ORIGINS = [
    'http://tauri.localhost',
    'https://tauri.localhost',
    'http://localhost:5190',
    'http://127.0.0.1:5190'
  ];
  var armed = false;

  function post(message) {
    for (var i = 0; i < PARENT_ORIGINS.length; i++) {
      try {
        window.parent.postMessage(message, PARENT_ORIGINS[i]);
      } catch (error) {
        /* 目标 origin 不匹配时浏览器静默丢弃；此处异常同样忽略。 */
      }
    }
  }

  function announce() {
    post({ __qxShell: TAG, kind: 'ready' });
  }

  window.addEventListener('message', function (event) {
    var data = event.data;
    if (
      data &&
      data[TAG] === TAG &&
      data.kind === 'arm' &&
      PARENT_ORIGINS.indexOf(event.origin) !== -1
    ) {
      armed = true;
      announce();
    }
  });

  // 启动即宣告：外壳收到 ready 才下发 arm。延迟脚本先于外壳 iframe load
  // 事件运行，正常时序是 ready → arm；这里再补一发 announce 保底幂等。
  announce();

  function dispatch(anchor, event) {
    if (!armed || event.defaultPrevented) return;
    var href = anchor.getAttribute('href');
    if (!href || href.charAt(0) === '#') return;
    var url;
    try {
      url = new URL(href, location.href);
    } catch (error) {
      return;
    }
    if (url.protocol !== 'http:' && url.protocol !== 'https:') return;
    if (url.origin === location.origin) return; // DSH 站内导航保持原生 SPA 行为
    event.preventDefault();
    event.stopPropagation();
    post({
      __qxShell: TAG,
      kind: 'open-url',
      url: url.href,
      ctrl: !!(event.ctrlKey || event.metaKey),
      middle: event.type === 'auxclick' && event.button === 1,
      shift: !!event.shiftKey
    });
  }

  function anchorOf(target) {
    if (target && typeof target.closest === 'function') {
      return target.closest('a[href]');
    }
    return null;
  }

  document.addEventListener(
    'click',
    function (event) {
      if (event.button !== 0) return;
      var anchor = anchorOf(event.target);
      if (anchor) dispatch(anchor, event);
    },
    true
  );
  // 中键 = 新窗打开（千寻内置窗级联新开）。
  document.addEventListener(
    'auxclick',
    function (event) {
      if (event.button !== 1) return;
      var anchor = anchorOf(event.target);
      if (anchor) dispatch(anchor, event);
    },
    true
  );
})();
