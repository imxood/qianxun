// 千寻移动端脚本示例 —— 复制到电脑的 <DSH_HOME>/mobile-access/custom.js 生效。
//
// 契约：window.qxMobile.register(({ root }) => { ...; return 清理函数 })
// - root 是挂载在页面 body 上的容器（定制内容一律放进 root，不要直接改 DSH 的节点结构）；
// - 返回的清理函数在热替换/卸载时被调用，必须撤销自己加的定时器、监听器和 DOM；
// - 涉及 DSH 界面节点的增强（如本例的滚动按钮）用 MutationObserver 等页面就绪后再挂。

window.qxMobile.register(({ root }) => {
  const button = document.createElement('button');
  button.type = 'button';
  button.textContent = '↑';
  button.setAttribute('aria-label', '回到顶部');
  button.style.cssText = [
    'position:fixed',
    'right:16px',
    'bottom:calc(24px + env(safe-area-inset-bottom))',
    'z-index:1200',
    'width:48px',
    'height:48px',
    'border-radius:50%',
    'border:none',
    'background:#3b82f6',
    'color:#fff',
    'font-size:20px',
    'box-shadow:0 4px 12px rgb(15 23 42 / 25%)',
  ].join(';');
  button.addEventListener('click', () => {
    const scroller = document.scrollingElement || document.documentElement;
    scroller.scrollTo({ top: 0, behavior: 'smooth' });
  });

  // 等首个消息区出现再显示，避免空会话时挡视线。
  const observer = new MutationObserver(() => {
    button.hidden = document.querySelectorAll('[class*="message"]').length === 0;
  });
  observer.observe(document.body, { childList: true, subtree: true });

  root.appendChild(button);

  return () => {
    observer.disconnect();
    button.remove();
  };
});
