/* 千寻移动端 · Android 软键盘适配
 * ─────────────────────────────────────────────────────────────
 * 配 mobile/examples/android-keyboard/custom.css 使用。
 * 契约：window.qxMobile.register(({ root }) => { ...; return 清理函数 })
 */

(() => {
  if (window.__qx_kb_dismiss_installed__) return;
  window.__qx_kb_dismiss_installed__ = true;

  window.qxMobile.register(({ root, document, window }) => {
    // 仅 Android + 软键盘场景触发；桌面 / iOS 静默 noop。
    const ua = navigator.userAgent;
    const isAndroid = /Android/i.test(ua);
    if (!isAndroid || !window.visualViewport) return () => {};

    // ── 1) 聚焦时主动滚到视口中央 ───────────────────────────────
    // DSH 聊天页若 fixed 定位输入框，adjustResize 不会自动重定位；
    // 我们监 focusin 主动滚——CSS 的 scroll-margin-bottom 把它送到中心。
    const onFocusIn = event => {
      const target = event.target;
      if (!(target instanceof HTMLElement)) return;
      const tag = target.tagName;
      const editable = target.isContentEditable;
      const isInput = /^(INPUT|TEXTAREA|SELECT)$/i.test(tag);
      if (!isInput && !editable) return;
      // 等 IME 完成弹起（约 100-300ms）再滚，避免在动画过程中滚动
      setTimeout(
        () => target.scrollIntoView({ block: 'center', behavior: 'smooth' }),
        250,
      );
    };
    document.addEventListener('focusin', onFocusIn);

    // ── 2) "收起键盘"按钮 ────────────────────────────────────────
    // Android WebView 通常没有原生"收起"按钮；浮一个在键盘上方。
    // 通过 visualViewport.height 变化检测键盘：可视区变小即键盘弹出。
    const btn = document.createElement('button');
    btn.id = 'qx-kb-dismiss';
    btn.type = 'button';
    btn.setAttribute('aria-label', '收起键盘');
    btn.textContent = '⌄';
    btn.addEventListener('click', () => {
      const active = document.activeElement;
      if (active instanceof HTMLElement) active.blur();
    });
    root.appendChild(btn);

    const vv = window.visualViewport;
    const update = () => {
      // 键盘弹出 → visualViewport 高度 < window.innerHeight - 100
      // 用 100px 阈值避免状态栏/地址栏变化误触。
      const shown = vv.height < window.innerHeight - 100;
      btn.dataset.visible = shown ? 'true' : 'false';
    };
    vv.addEventListener('resize', update);
    vv.addEventListener('scroll', update);
    update();

    return () => {
      document.removeEventListener('focusin', onFocusIn);
      vv.removeEventListener('resize', update);
      vv.removeEventListener('scroll', update);
      btn.remove();
    };
  });
})();
