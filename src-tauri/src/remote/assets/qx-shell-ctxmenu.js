/*
 * 千寻桌面外壳层 —— 右键菜单屏蔽 + 自绘小菜单（回环网关注入，
 * 只注入千寻外壳承载的 DSH iframe）。
 *
 * 背景（Linux/CEF 实测）：CEF 的原生右键菜单是一个独立的
 * override-redirect X11 弹窗，被算成与主窗口等大（1100x734）⇒ 用户看到
 * 「巨大候选框」。前端在顶层页用 contextmenu + preventDefault() 已能
 * 压掉它，但 DSH 聊天页跑在 iframe 里（千寻回环网关代理的
 * http://127.0.0.1:23091/），前端方案管不到 ⇒ 由这里兜底。
 *
 * 约定（与 qx-shell-links.js 同风格）：
 * - 只在「嵌在千寻外壳里」（window.parent !== window）时接管；用户直接用
 *   浏览器打开网关地址时完全原生右键菜单，零破坏。
 * - preventDefault 是 contextmenu 事件的**第一动作**：先压掉原生大菜单，
 *   再决定要不要自绘小菜单。
 * - 自绘菜单是纯 DOM，不依赖千寻前端：复制/剪切/粘贴/全选；剪贴板走
 *   navigator.clipboard，退化到 document.execCommand，失败一律静默
 *   （不弹窗、不打断、不冒泡到外壳）。
 */
(function () {
  'use strict';
  if (window.__qxShellCtxMenu) return;
  window.__qxShellCtxMenu = true;

  var TAG = '__qxShell';
  // 可信父页 origin（千寻外壳）：release tauri.localhost / dev vite 5190。
  var PARENT_ORIGINS = [
    'http://tauri.localhost',
    'https://tauri.localhost',
    'http://localhost:5190',
    'http://127.0.0.1:5190'
  ];

  function post(message) {
    for (var i = 0; i < PARENT_ORIGINS.length; i++) {
      try {
        window.parent.postMessage(message, PARENT_ORIGINS[i]);
      } catch (error) {
        /* 目标 origin 不匹配时浏览器静默丢弃；此处异常同样忽略。 */
      }
    }
  }

  // 只在 iframe 里接管：顶层打开网关页保持原生右键菜单。
  var inShell = false;
  try {
    inShell = window.parent !== window;
  } catch (error) {
    inShell = false;
  }
  if (!inShell) return;

  // 告知外壳：本层已就位（外壳不认识这个 kind 也不影响，两边都只当消息看）。
  post({ __qxShell: TAG, kind: 'ctxmenu' });

  var currentMenu = null;

  // ---- 目标判定 ----

  function isEditable(node) {
    if (!node || node.nodeType !== 1) return false;
    var tag = (node.tagName || '').toLowerCase();
    if (tag === 'textarea') return true;
    if (tag === 'input') {
      var type = (node.getAttribute('type') || 'text').toLowerCase();
      return [
        'button',
        'submit',
        'reset',
        'checkbox',
        'radio',
        'hidden',
        'file',
        'image',
        'range',
        'color'
      ].indexOf(type) === -1;
    }
    return node.isContentEditable === true;
  }

  // 从事件目标往上找最近的可编辑祖先（输入框 / textarea / contenteditable）。
  function editableOf(node) {
    var current = node;
    var depth = 0;
    while (current && current.nodeType === 1 && depth < 8) {
      if (isEditable(current)) return current;
      current = current.parentElement;
      depth++;
    }
    return null;
  }

  // 当前选区文本：输入框/文本域读 selectionStart..selectionEnd，
  // 其余读 window.getSelection()。
  function currentSelection(editable) {
    if (
      editable &&
      typeof editable.selectionStart === 'number' &&
      typeof editable.selectionEnd === 'number'
    ) {
      if (editable.selectionEnd > editable.selectionStart) {
        return String(editable.value || '').slice(
          editable.selectionStart,
          editable.selectionEnd
        );
      }
      return '';
    }
    var selection = window.getSelection ? window.getSelection() : null;
    return selection ? String(selection.toString() || '') : '';
  }

  // ---- 剪贴板动作（失败静默） ----

  function exec(command, value) {
    try {
      if (value === undefined) return document.execCommand(command);
      return document.execCommand(command, false, value);
    } catch (error) {
      return false;
    }
  }

  function copyText(text) {
    var done = false;
    if (navigator.clipboard && navigator.clipboard.writeText) {
      try {
        var result = navigator.clipboard.writeText(text);
        if (result && typeof result.then === 'function') {
          // 拒绝（无焦点/无权限）时静默——不打断用户。
          result.then(function () {}, function () {});
        }
        done = true;
      } catch (error) {
        done = false;
      }
    }
    if (!done) {
      // 退化路径：靠当前选区走 execCommand('copy')。
      var selection = window.getSelection ? window.getSelection() : null;
      if (selection && selection.rangeCount > 0) exec('copy');
    }
  }

  function pasteInto(editable) {
    if (!editable) return;
    if (!navigator.clipboard || !navigator.clipboard.readText) {
      exec('paste');
      return;
    }
    try {
      navigator.clipboard.readText().then(
        function (text) {
          if (!text) return;
          editable.focus();
          // insertText 保留撤销栈，是首选。
          if (exec('insertText', text)) return;
          if (typeof editable.value === 'string') {
            var start = editable.selectionStart;
            var end = editable.selectionEnd;
            if (typeof start === 'number' && typeof end === 'number') {
              editable.value =
                String(editable.value).slice(0, start) +
                text +
                String(editable.value).slice(end);
              editable.selectionStart = editable.selectionEnd =
                start + text.length;
              try {
                editable.dispatchEvent(
                  new Event('input', { bubbles: true })
                );
              } catch (error) {
                /* 老浏览器没有 Event 构造器：忽略。 */
              }
            }
            return;
          }
          exec('insertText', text);
        },
        function () {
          /* 读取被拒（无权限）：静默。 */
        }
      );
    } catch (error) {
      /* 不支持 clipboard API：静默。 */
    }
  }

  function cutText(editable, text) {
    if (!text) return;
    copyText(text);
    if (editable) exec('delete');
  }

  function selectAll(editable) {
    if (!editable) return;
    editable.focus();
    if (typeof editable.select === 'function') {
      try {
        editable.select();
        return;
      } catch (error) {
        /* 落到 execCommand。 */
      }
    }
    exec('selectAll');
  }

  // ---- 自绘菜单 ----

  function hideMenu() {
    if (currentMenu && currentMenu.parentNode) {
      currentMenu.parentNode.removeChild(currentMenu);
    }
    currentMenu = null;
  }

  function buildItems(editable, selected) {
    var items = [];
    if (editable) {
      if (selected) {
        items.push({
          label: '复制',
          action: function () {
            copyText(selected);
          }
        });
        items.push({
          label: '剪切',
          action: function () {
            cutText(editable, selected);
          }
        });
      }
      items.push({
        label: '粘贴',
        action: function () {
          pasteInto(editable);
        }
      });
      items.push({
        label: '全选',
        action: function () {
          selectAll(editable);
        }
      });
      return items;
    }
    if (selected) {
      items.push({
        label: '复制',
        action: function () {
          copyText(selected);
        }
      });
    }
    return items;
  }

  function showMenu(x, y, editable) {
    hideMenu();
    var selected = currentSelection(editable);
    var items = buildItems(editable, selected);
    // 无可操作内容：不显示任何东西（原生菜单已被 preventDefault 压掉）。
    if (!items.length) return;

    var menu = document.createElement('div');
    menu.id = '__qx-shell-ctxmenu';
    menu.setAttribute('role', 'menu');
    menu.style.cssText = [
      'position:fixed',
      'z-index:2147483647',
      'min-width:128px',
      'padding:4px 0',
      'margin:0',
      'border:1px solid rgba(0,0,0,.18)',
      'border-radius:8px',
      'background:#ffffff',
      'color:#1f2328',
      'box-shadow:0 6px 20px rgba(0,0,0,.18)',
      'font:13px/1.4 system-ui,-apple-system,"Segoe UI",sans-serif',
      'user-select:none',
      '-webkit-user-select:none'
    ].join(';');

    items.forEach(function (item) {
      var button = document.createElement('div');
      button.setAttribute('role', 'menuitem');
      button.textContent = item.label;
      button.style.cssText = [
        'padding:5px 14px',
        'cursor:default',
        'white-space:nowrap'
      ].join(';');
      button.addEventListener('mouseenter', function () {
        button.style.background = '#e8f0fe';
      });
      button.addEventListener('mouseleave', function () {
        button.style.background = 'transparent';
      });
      button.addEventListener('mousedown', function (event) {
        // mousedown 早于 blur：拦掉默认，避免输入框失焦导致选区丢失。
        event.preventDefault();
      });
      button.addEventListener('click', function (event) {
        event.preventDefault();
        event.stopPropagation();
        hideMenu();
        try {
          item.action();
        } catch (error) {
          /* 动作失败静默。 */
        }
        post({ __qxShell: TAG, kind: 'ctxmenu', action: item.label });
      });
      menu.appendChild(button);
    });

    if (!document.body) return;
    document.body.appendChild(menu);
    currentMenu = menu;

    // 贴边收拢：超出视口时向内收，避免菜单被裁掉。
    var rect = menu.getBoundingClientRect();
    var maxLeft = Math.max(4, window.innerWidth - rect.width - 4);
    var maxTop = Math.max(4, window.innerHeight - rect.height - 4);
    menu.style.left = Math.min(x, maxLeft) + 'px';
    menu.style.top = Math.min(y, maxTop) + 'px';

    post({ __qxShell: TAG, kind: 'ctxmenu', state: 'open' });
  }

  // ---- 事件接线 ----

  document.addEventListener(
    'contextmenu',
    function (event) {
      // **第一动作**：压掉 CEF 原生右键菜单（override-redirect 弹窗
      // 被算成整窗大小 ⇒ 巨大候选框）。后面的一切都不能先于它。
      event.preventDefault();
      if (!inShell) return;
      showMenu(event.clientX, event.clientY, editableOf(event.target));
    },
    true
  );

  // 点菜单外面 → 收起（capture 阶段，但菜单自身命中时不收）。
  document.addEventListener(
    'mousedown',
    function (event) {
      if (!currentMenu) return;
      var target = event.target;
      if (target && currentMenu.contains(target)) return;
      hideMenu();
    },
    true
  );

  document.addEventListener(
    'keydown',
    function (event) {
      if (event.key === 'Escape') hideMenu();
    },
    true
  );

  window.addEventListener('blur', hideMenu);
  window.addEventListener('resize', hideMenu);
  document.addEventListener('scroll', hideMenu, true);
})();
