/* 千寻移动端 · 最小可用定制层（custom.js）
 * ─────────────────────────────────────────────────────────────
 * 部署到：<DSH_HOME>/mobile-access/custom.js
 *
 * 契约：window.qxMobile.register(({ root }) => { ...; return 清理函数 })
 *
 * ============================== 范围纪律 ==============================
 * 本文件**只做一件事**：让「输入框 + 软键盘」在手机上可用。
 * ⛔ **不做任何视觉美化**——不建覆盖层、不改 DOM 结构、不改配色。
 *    （历史 L3 定制层里的 qx-layer / qx-sheet 等自绘 UI 已整体下线。）
 *
 * 入口必须是**函数表达式**，不能是立即执行的 async 箭头
 * （window.qxMobile.register((async (ctx) => {...})() 会在某些
 *  WebView 上表现为「界面完全不动」，语法却合法）。
 */
(() => {
  if (window.__qx_input_kb_installed__) return;
  window.__qx_input_kb_installed__ = true;

  window.qxMobile.register(({ root, document, window }) => {
    // ── 0) 自举 CSS（绕开定制层的热替换失效）──────────────────────
    //
    // 实测（2026-11-05，OnePlus PKB110 / Android 16 / 正式版千寻 11-04 构建）：
    // 网关把 /qx-mobile/custom.css 以
    //   `cache-control: public, max-age=31536000, immutable` 提供
    // （src-tauri/src/remote/mobile_ui.rs:248 CUSTOM_CACHE_CONTROL），
    // 而 HTML 注入的那一行是**固定 URL、无版本参数**
    // （src-tauri/src/remote/gateway.rs:521 MOBILE_LAYER_SNIPPET）。
    // 两者叠加 = 这张样式表**必然长期命中陈旧缓存**。
    //
    // 现象（真机抓 stylesheet 列表）：页面上只有 DSH 自己的两张
    //   ./assets/vendor-*.css / ./assets/index-*.css
    // 与本文件注入的这一张，**没有** bootstrap 自己那张带 hash 参数的
    // <link> —— 说明它的 applyCss() 从未执行，tick() 静默失败。
    // 于是文档里写的「5 秒自动生效」实际不成立。
    //
    // 这里的做法：自己挂一份**带时间戳 query** 的样式表。
    // query 不同 -> 缓存键不同 -> 一定重新下载，一定拿到最新 CSS。
    // 只移除「无 query」的那一份（HTML 注入的、带旧缓存），
    // 保留 bootstrap 的（若它将来恢复工作，两者并存也无害）。
    //
    // ⛔ 千寻侧待修（见 mobile-flutter/docs/design/24-移动端问题修复记录.md §8）：
    //    custom.css 的缓存策略 + version 端点在 LAN 入口的鉴权，
    //    修好后本段可删。
    try {
      for (const link of [
        ...document.querySelectorAll('link[rel="stylesheet"]'),
      ]) {
        const href = link.getAttribute('href') || '';
        if (href.includes('/qx-mobile/custom.css') && !href.includes('?')) {
          link.remove();
        }
      }
      const link = document.createElement('link');
      link.rel = 'stylesheet';
      link.dataset.qxMobileSelfBoot = 'css';
      link.href = '/qx-mobile/custom.css?v=' + Date.now();
      document.head.appendChild(link);
    } catch (e) {
      // 自举失败不影响软键盘功能
    }

    // 非 Android / 无 visualViewport（桌面、iOS、旧 WebView）：静默 noop。
    const isAndroid = /Android/i.test(navigator.userAgent);
    if (!isAndroid || !window.visualViewport) return () => {};

    // ── 1) 聚焦时把输入框从键盘下面挪出来 ─────────────────────────
    //
    // ⚠️ 2026-11-05 真机时间线实测（OnePlus PKB110 / Android 16）后重写。
    // 旧写法（固定 250ms + smooth + scroll-margin-bottom:50vh）会带来
    // 「输入法上浮过程发滞」的观感，根因有三：
    //
    //   a) **时机错位**：固定 250ms 落在键盘动画**进行中**
    //      （实测键盘动画 t=117ms 起、t=423ms 止）。此时发起滚动，
    //      滚动动画与键盘动画**叠加**，输入框走两段路 -> 迟滞。
    //   b) **二次滚动**：若页面（或 DSH 的对话区）有内部滚动容器，
    //      scrollIntoView 会再滚一次，内容跟着跳一下。
    //   c) **位移过大**：block:'center' + scroll-margin-bottom:50vh
    //      把目标推到视口中部，滚动量是必要的数倍。
    //
    // 实测还证明了一件事：**这台机器上根本不需要滚动**——
    // `visualViewport.height ≈ window.innerHeight`（693/692 -> 393/392），
    // 说明布局视口随键盘一起缩小（Flutter 的 resizeToAvoidBottomInset
    // 等效于 adjustResize），输入框在文档流末尾，**自动贴着新视口底边**，
    // 位移量(574->274)与视口缩小量(693->393)完全一致 —— 天然跟手。
    //
    // 所以新写法是**按需补偿**：
    //   1) 等键盘动画**稳定**（连续两帧 visualViewport 不变），而不是猜时长；
    //   2) 只有输入框**确实被键盘遮住**时才滚动；
    //   3) 滚动用 'nearest' + 非平滑 -> 最小位移、一次到位。
    const isOccluded = el => {
      const r = el.getBoundingClientRect();
      // 优先用壳通道的键盘高度；无通道时退回 visualViewport
      const kb = window.__qxKbHeight || 0;
      const visibleBottom =
        kb > 0
          ? window.innerHeight - kb
          : window.visualViewport.height +
            (window.visualViewport.offsetTop || 0);
      return r.bottom > visibleBottom + 8;
    };

    // 键盘动画稳定后回调（连续两帧 vv 不变即认为结束；800ms 兜底）
    const afterKeyboardSettled = cb => {
      let last = -1;
      let stable = 0;
      const t0 = performance.now();
      const tick = () => {
        const h = window.visualViewport.height;
        if (h === last) {
          stable += 1;
        } else {
          stable = 0;
          last = h;
        }
        if (stable >= 2 || performance.now() - t0 > 800) {
          cb();
          return;
        }
        requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    };

    // ── 1.5) 键盘高度通道：逐帧跟手且零卡顿 ─────────────────────
    //
    // 配合 Flutter 壳（dsh_page.dart）：壳不再 resize WebView
    // （resizeToAvoidBottomInset=false -> surface 尺寸全程不变 -> 键盘
    // 动画由系统独立绘制，A/B 实测长帧 8~9 个 -> 0 个），改为把 Android 11+
    // 的**逐帧** IME inset 推进来：window.__qxKb(高度px)。
    //
    // 跟手由这里完成，全部是**廉价操作**：
    //   a) composerSeat 用 transform 上移 —— compositor-only，
    //      不触发布局/光栅化（组件 ~127px 高，独占小图层）；
    //   b) 键盘高度**稳定后**给滚动容器一次性补 padding-bottom=kb，
    //      让「滚到底」时消息尾行停在 composer 上方（座位是滚动容器的
    //      流内末尾元素，若不补，流式输出的最新内容会滚进
    //      「键盘+composer」的遮挡带后面）。
    //
    // 选择器纪律：座位**运行时推导** —— 从 composer 槽向上找第一个
    // overflow:auto/scroll 的滚动容器，再取它与滚动容器之间的那个子元素
    // （DSH 的哈希 class 每次构建都变，不能写死）。
    window.__qxKbHeight = 0;
    let kbRaf = 0;
    let kbPadKey = -1;
    let seatEl = null;
    let scrollerEl = null;
    const ensureKbTargets = () => {
      const composer = document.querySelector(
        "[data-slot='conversation.composer']",
      );
      if (!composer) return;
      let scroller = null;
      for (
        let el = composer.parentElement;
        el && el !== document.body;
        el = el.parentElement
      ) {
        const oy = getComputedStyle(el).overflowY;
        if (oy === 'auto' || oy === 'scroll') {
          scroller = el;
          break;
        }
      }
      if (!scroller) return;
      let seat = composer;
      while (seat.parentElement && seat.parentElement !== scroller) {
        seat = seat.parentElement;
      }
      scrollerEl = scroller;
      seatEl = seat;
    };
    // transform 按**模式**计算（⛔ 不能无脑 -kb，否则 hide 首帧会把
    // 已被 padding 抬到键盘上沿的 composer 再上移一个 kb，瞬移到顶栏
    // 后面 —— 真机实测的「输入法消失时输入框向上弹一下」就是这个）：
    //   a) padding 未加（show 动画中）：ty = -kb，composer 从屏幕底爬升；
    //   b) padding 已加（settle 后 / hide 动画中）：ty = pad - kb ——
    //      hide 时 ty 从 0 连续长到 +pad，composer 贴着键盘上沿下降，
    //      kb=0 清 padding 后正好落在流内底部，全程无跳变。
    const applyKb = kb => {
      ensureKbTargets();
      if (!seatEl) return;
      if (kb <= 0) {
        // 键盘完全收起：padding 与 transform 同帧清零，
        // composer 回到流内底部（= 无键盘的常态位置）。
        if (kbPadKey !== 0) {
          kbPadKey = 0;
          if (scrollerEl) scrollerEl.style.paddingBottom = '';
        }
        seatEl.style.transform = '';
      } else {
        const ty = kbPadKey > 0 ? kbPadKey - kb : -kb;
        seatEl.style.transform = `translateY(${ty}px)`;
      }
      document.body.dataset.qxKeyboard = kb > 0 ? 'shown' : 'hidden';
    };
    window.__qxKb = h => {
      const kb = Math.max(0, Number(h) || 0);
      window.__qxKbHeight = kb;
      if (!kbRaf) {
        kbRaf = requestAnimationFrame(() => {
          kbRaf = 0;
          applyKb(window.__qxKbHeight);
        });
      }
      if (kb <= 0) return;
      // 高度稳定后一次性补滚动余量（不逐帧 layout）。
      // ⚠️ padding 让座位的**流内位置**正好停在键盘上沿，
      // 所以同时必须清掉 transform（applyKb 的模式 b 从下一帧接管，
      // 视觉位置连续）。
      afterKeyboardSettled(() => {
        if (window.__qxKbHeight === kb && kbPadKey !== kb) {
          kbPadKey = kb;
          ensureKbTargets();
          if (scrollerEl) scrollerEl.style.paddingBottom = `${kb}px`;
          if (seatEl) seatEl.style.transform = '';
        }
      });
    };

    const onFocusIn = event => {
      const target = event.target;
      if (!(target instanceof HTMLElement)) return;
      const editable = target.isContentEditable;
      const isInput = /^(INPUT|TEXTAREA|SELECT)$/i.test(target.tagName);
      if (!isInput && !editable) return;
      afterKeyboardSettled(() => {
        // 系统没替我们挪（输入框仍被键盘盖住）才补一刀，且是最小位移。
        if (isOccluded(target)) {
          target.scrollIntoView({ block: 'nearest', behavior: 'auto' });
        }
      });
    };
    document.addEventListener('focusin', onFocusIn);

    // ── 2) 键盘弹起时给 body 打标记（供 CSS 收窄顶部留白）─────────
    // visualViewport 高度显著变小 = 键盘弹起。
    // ⚠️ 阈值 100px：避开状态栏/地址栏的常规变化误触。
    const vv = window.visualViewport;
    const onViewportChange = () => {
      const keyboardShown = vv.height < window.innerHeight - 100;
      document.body.dataset.qxKeyboard = keyboardShown ? 'shown' : 'hidden';
    };
    vv.addEventListener('resize', onViewportChange);
    vv.addEventListener('scroll', onViewportChange);
    onViewportChange();

    return () => {
      document.removeEventListener('focusin', onFocusIn);
      vv.removeEventListener('resize', onViewportChange);
      vv.removeEventListener('scroll', onViewportChange);
      delete document.body.dataset.qxKeyboard;
      delete window.__qxKb;
      delete window.__qxKbHeight;
      if (seatEl) seatEl.style.transform = '';
      if (scrollerEl) scrollerEl.style.paddingBottom = '';
    };
  });
})();
