<script lang="ts">
  /**
   * 截屏覆盖窗（#/overlay?monitor=N&path=…）：微信式交互。
   * 冻结帧做底图 → 暗化遮罩 + 十字光标 → 拖框选区（可移动/8 手柄微调）
   * → 标注工具栏（矩形/椭圆/箭头/画笔/马赛克/文字 + 撤销重做）
   * → 产出（复制/保存/贴图/退出）。双击选区或按回车 = 复制并退出；Esc = 退出。
   */
  import { onMount, tick } from 'svelte';
  import { convertFileSrc } from '@tauri-apps/api/core';
  import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
  import { call } from '../../lib/ipc';

  // ---- URL 参数 ----
  const params = new URLSearchParams(window.location.hash.split('?')[1] ?? '');
  const imagePath = params.get('path') ?? '';
  /** 本屏原点（虚拟屏幕物理像素）：窗口感知的 IPC 探点换算用。 */
  const monitorX = Number(params.get('x') ?? '0');
  const monitorY = Number(params.get('y') ?? '0');

  // ---- DOM ----
  let canvas: HTMLCanvasElement | null = $state(null);
  let baseImage = $state<HTMLImageElement | null>(null);

  // ---- 坐标系：全部物理像素（canvas.width = 图像物理宽）----
  let dpr = $state(1);

  type Tool = 'none' | 'rect' | 'ellipse' | 'arrow' | 'pen' | 'mosaic' | 'text';
  type Rect = { x1: number; y1: number; x2: number; y2: number };
  type WindowHit = { x: number; y: number; width: number; height: number; title: string };

  type Shape =
    | { type: 'rect'; rect: Rect; color: string; width: number }
    | { type: 'ellipse'; rect: Rect; color: string; width: number }
    | {
        type: 'arrow';
        from: { x: number; y: number };
        to: { x: number; y: number };
        color: string;
        width: number;
      }
    | { type: 'pen'; points: Array<{ x: number; y: number }>; color: string; width: number }
    | { type: 'mosaic'; rect: Rect }
    | { type: 'text'; at: { x: number; y: number }; text: string; color: string; size: number };

  // ---- 会话状态 ----
  let phase = $state<'idle' | 'selecting' | 'selected'>('idle');
  let selection = $state<Rect>({ x1: 0, y1: 0, x2: 0, y2: 0 });
  /** 微信式窗口感知：悬停命中的窗口矩形（屏内物理像素；idle 态才生效）。 */
  let hoverRect = $state<Rect | null>(null);
  /** 感知节流：上一次 IPC 探测时刻与探点（鼠标高频移动时压调用频次）。 */
  let hoverProbeAt = 0;
  let hoverProbePoint: Pt = { x: -1, y: -1 };
  let tool = $state<Tool>('none');
  let color = $state('#E53935');
  let strokeWidth = $state(4);
  let shapes = $state<Shape[]>([]);
  let redoStack = $state<Shape[]>([]);
  // 当前选中的标注（shapes 下标）：「选择」工具下可拖动 / 缩放 / 删除。
  let selected = $state<number | null>(null);
  let message = $state('');
  let working = $state(false);

  // 拖拽上下文（不参与渲染，普通变量即可）
  type Pt = { x: number; y: number };
  type Drag =
    | { kind: 'new'; origin: Rect; start: Pt }
    | { kind: 'move'; origin: Rect; start: Pt }
    | { kind: 'resize'; handle: number; origin: Rect; start: Pt }
    | { kind: 'shape-move'; index: number; origin: Shape; start: Pt }
    | { kind: 'shape-resize'; index: number; handle: number; origin: Shape; start: Pt };
  let drag: Drag | null = null;
  let liveShape: Shape | null = null;
  /** mousedown 瞬间的感知命中：原地松开（≈未拖动）= 接受窗口选区。 */
  let pressHover: Rect | null = null;
  let textDraft = $state<{ x: number; y: number; value: string } | null>(null);

  // 'none' = 默认空态（可拖选区/编辑标注），不再是按钮；再点激活的工具可切回。
  const TOOLS: Array<{ id: Tool; label: string; icon: string }> = [
    { id: 'rect', label: '矩形', icon: 'M4 5h16v14H4z' },
    { id: 'ellipse', label: '椭圆', icon: 'M12 5a8 7 0 100 14 8 7 0 000-14z' },
    { id: 'arrow', label: '箭头', icon: 'M4 20L20 4M20 4h-7M20 4v7' },
    { id: 'pen', label: '画笔', icon: 'M3 21c3 0 5-1 7-3l9-9-4-4-9 9c-2 2-3 4-3 7z' },
    {
      id: 'mosaic',
      label: '马赛克',
      icon: 'M4 4h4v4H4zM10 4h4v4h-4zM16 4h4v4h-4zM4 10h4v4H4zM10 10h4v4h-4zM16 10h4v4h-4zM4 16h4v4H4zM10 16h4v4h-4zM16 16h4v4h-4z',
    },
    { id: 'text', label: '文字', icon: 'M5 5h14M12 5v14M8 19h8' },
  ];
  const COLORS = ['#E53935', '#FDD835', '#1E88E5', '#43A047', '#111827', '#FFFFFF'];
  const WIDTHS = [2, 4, 8];

  // ---- 工具栏定位（CSS 像素，HTML 层）----
  // toolbarBase = 贴着选区的默认位置；toolbarBox = 默认位置 + 用户拖动偏移
  // （钳制在屏内）。新选区开始时偏移归位（重新贴到选区旁）。
  let toolbarW = $state(0);
  let toolbarOffset = $state({ x: 0, y: 0 });
  let colorOpen = $state(false);
  let widthOpen = $state(false);
  let toolbarDrag: { sx: number; sy: number; ox: number; oy: number } | null = null;
  const TOOLBAR_FALLBACK_W = 300;

  const toolbarBase = $derived.by(() => {
    if (phase !== 'selected') return null;
    const phys = norm(selection);
    const w = toolbarW || TOOLBAR_FALLBACK_W;
    const h = 44;
    let x = phys.x1 / dpr;
    let y = phys.y2 / dpr + 12;
    const cssW = canvas?.clientWidth ?? 0;
    const cssH = canvas?.clientHeight ?? 0;
    if (y + h > cssH - 8) y = phys.y1 / dpr - h - 12;
    if (y < 8) y = 8;
    x = Math.min(Math.max(x, 8), Math.max(8, cssW - w - 8));
    return { x, y, w, h };
  });

  const toolbarBox = $derived.by(() => {
    const base = toolbarBase;
    if (!base) return null;
    const cssW = canvas?.clientWidth ?? 0;
    const cssH = canvas?.clientHeight ?? 0;
    const x = Math.min(Math.max(base.x + toolbarOffset.x, 8), Math.max(8, cssW - base.w - 8));
    const y = Math.min(Math.max(base.y + toolbarOffset.y, 8), Math.max(8, cssH - base.h - 8));
    return { x, y };
  });

  /** 收起颜色 / 线宽下拉（点外部、Esc、回车产出时）。 */
  function closeMenus(): void {
    colorOpen = false;
    widthOpen = false;
  }

  const sizeLabel = $derived.by(() => {
    if (phase === 'idle') return '';
    const phys = norm(selection);
    return `${Math.round(Math.abs(phys.x2 - phys.x1))} × ${Math.round(Math.abs(phys.y2 - phys.y1))}`;
  });

  function norm(rect: Rect): Rect {
    return {
      x1: Math.min(rect.x1, rect.x2),
      y1: Math.min(rect.y1, rect.y2),
      x2: Math.max(rect.x1, rect.x2),
      y2: Math.max(rect.y1, rect.y2),
    };
  }

  // ---- 初始化 ----
  // 首帧就绪上报：覆盖窗默认隐藏，全部屏画好第一帧后由 Rust 统一亮窗，
  // 消除热键后 WebView 未绘制 / 底图未解码的一瞬黑屏。
  let readySent = false;
  function signalReady(): void {
    if (readySent) return;
    readySent = true;
    // 不用 rAF 确认合成：窗口隐藏期间 WebView 暂停渲染管线，rAF 不会触发，
    // 只能干等兜底。画布位图在 show 时由合成器直接呈现，画完即报即可。
    void call('shots_overlay_ready', { label: getCurrentWebviewWindow().label }).catch(() => {});
  }

  onMount(() => {
    dpr = window.devicePixelRatio || 1;
    const image = new Image();
    // asset protocol 相对页面是跨源：不带 crossOrigin 时画布会被污染，
    // 导出 toDataURL 报 "Tainted canvases may not be exported"。
    // asset protocol 返回 Access-Control-Allow-Origin，匿名 CORS 拉取即可保持画布干净。
    image.crossOrigin = 'anonymous';
    image.onload = () => {
      baseImage = image;
      // 先等 Svelte 把底图尺寸写进 canvas 再绘制：状态更新是异步落 DOM，
      // 立即 render() 会画在 0×0 画布上，随后尺寸落位又把画布清空 → 亮窗全黑。
      void tick().then(() => {
        render();
        signalReady();
      });
    };
    // 底图缺失也要上报亮窗：用户至少还能框选或 Esc 退出。
    image.onerror = () => signalReady();
    image.src = convertFileSrc(imagePath);
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        if (textDraft) {
          textDraft = null;
          return;
        }
        // 下拉开着先收起，不当作退出意图。
        if (colorOpen || widthOpen) {
          closeMenus();
          return;
        }
        // 先取消标注选中，再退出截屏。
        if (tool === 'none' && selected !== null) {
          selected = null;
          render();
          return;
        }
        void exitAll();
      } else if (event.key === 'Enter') {
        // 选区就绪后回车 = 复制并退出（文字草稿中的回车归输入框落墨）。
        // preventDefault 顺带抑制焦点工具栏按钮的回车激活，避免双重动作。
        if (textDraft || phase !== 'selected') return;
        event.preventDefault();
        void produce('copy');
      } else if (
        (event.key === 'Delete' || event.key === 'Backspace') &&
        tool === 'none' &&
        selected !== null
      ) {
        event.preventDefault();
        const target = selected;
        shapes = shapes.filter((_, index) => index !== target);
        redoStack = [];
        selected = null;
        render();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  // ---- 坐标换算 ----
  function toPhysical(event: MouseEvent): { x: number; y: number } {
    const bounds = canvas!.getBoundingClientRect();
    const scaleX = canvas!.width / bounds.width;
    const scaleY = canvas!.height / bounds.height;
    return {
      x: (event.clientX - bounds.left) * scaleX,
      y: (event.clientY - bounds.top) * scaleY,
    };
  }

  // ---- 渲染 ----
  function render(): void {
    if (!canvas || !baseImage) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    ctx.drawImage(baseImage, 0, 0);

    // 暗化遮罩：选区外（未选区时全屏）；idle 态有窗口感知时改为窗口外。
    const hasSelection = phase !== 'idle';
    const sel = hasSelection ? norm(selection) : null;
    const hover = phase === 'idle' && hoverRect ? norm(hoverRect) : null;
    ctx.save();
    ctx.fillStyle = 'rgba(0, 0, 0, 0.45)';
    ctx.beginPath();
    ctx.rect(0, 0, canvas.width, canvas.height);
    if (sel) ctx.rect(sel.x1, sel.y1, sel.x2 - sel.x1, sel.y2 - sel.y1);
    else if (hover) ctx.rect(hover.x1, hover.y1, hover.x2 - hover.x1, hover.y2 - hover.y1);
    ctx.fill('evenodd');
    ctx.restore();

    // 感知高亮：天蓝描边（微信语义），窗口边界一目了然。
    if (hover) {
      ctx.save();
      ctx.lineWidth = 2 * dpr;
      ctx.strokeStyle = 'rgba(56, 189, 248, 0.95)';
      ctx.strokeRect(hover.x1, hover.y1, hover.x2 - hover.x1, hover.y2 - hover.y1);
      ctx.restore();
    }

    if (sel) {
      // 选区边框（白色 + 内衬深色，任何底色可见）。
      ctx.save();
      ctx.lineWidth = 1 * dpr;
      ctx.strokeStyle = 'rgba(255,255,255,0.9)';
      ctx.strokeRect(sel.x1 + 0.5, sel.y1 + 0.5, sel.x2 - sel.x1 - 1, sel.y2 - sel.y1 - 1);
      ctx.restore();

      // 标注（裁剪进选区）+ 进行中的形状。
      ctx.save();
      ctx.beginPath();
      ctx.rect(sel.x1, sel.y1, sel.x2 - sel.x1, sel.y2 - sel.y1);
      ctx.clip();
      for (const shape of shapes) drawShape(ctx, shape);
      if (liveShape) drawShape(ctx, liveShape);
      ctx.restore();
    }

    // 选中标注的编辑框：虚线框 + 可缩放形状的 8 手柄（仅「选择」工具下显示）。
    if (phase === 'selected' && tool === 'none' && selected !== null) {
      const shape = shapes[selected];
      if (shape) {
        const box = outlineBox(shape);
        ctx.save();
        ctx.strokeStyle = 'rgba(255,255,255,0.95)';
        ctx.lineWidth = 1 * dpr;
        ctx.setLineDash([5 * dpr, 4 * dpr]);
        ctx.strokeRect(box.x1, box.y1, box.x2 - box.x1, box.y2 - box.y1);
        ctx.setLineDash([]);
        if (shapeResizable(shape)) {
          const size = 7 * dpr;
          ctx.fillStyle = '#ffffff';
          ctx.strokeStyle = 'rgba(0,0,0,0.45)';
          for (const [x, y] of boxHandlePositions(box)) {
            ctx.fillRect(x - size / 2, y - size / 2, size, size);
            ctx.strokeRect(x - size / 2, y - size / 2, size, size);
          }
        }
        ctx.restore();
      }
    }
  }

  function drawShape(ctx: CanvasRenderingContext2D, shape: Shape): void {
    ctx.lineCap = 'round';
    ctx.lineJoin = 'round';
    switch (shape.type) {
      case 'rect': {
        const rect = norm(shape.rect);
        ctx.strokeStyle = shape.color;
        ctx.lineWidth = shape.width;
        ctx.strokeRect(rect.x1, rect.y1, rect.x2 - rect.x1, rect.y2 - rect.y1);
        break;
      }
      case 'ellipse': {
        const rect = norm(shape.rect);
        ctx.strokeStyle = shape.color;
        ctx.lineWidth = shape.width;
        ctx.beginPath();
        ctx.ellipse(
          (rect.x1 + rect.x2) / 2,
          (rect.y1 + rect.y2) / 2,
          Math.abs(rect.x2 - rect.x1) / 2,
          Math.abs(rect.y2 - rect.y1) / 2,
          0,
          0,
          Math.PI * 2,
        );
        ctx.stroke();
        break;
      }
      case 'arrow': {
        const { from, to } = shape;
        ctx.strokeStyle = shape.color;
        ctx.fillStyle = shape.color;
        ctx.lineWidth = shape.width;
        ctx.beginPath();
        ctx.moveTo(from.x, from.y);
        ctx.lineTo(to.x, to.y);
        ctx.stroke();
        const angle = Math.atan2(to.y - from.y, to.x - from.x);
        const head = Math.max(shape.width * 4, 12);
        ctx.beginPath();
        ctx.moveTo(to.x, to.y);
        ctx.lineTo(
          to.x - head * Math.cos(angle - Math.PI / 6),
          to.y - head * Math.sin(angle - Math.PI / 6),
        );
        ctx.lineTo(
          to.x - head * Math.cos(angle + Math.PI / 6),
          to.y - head * Math.sin(angle + Math.PI / 6),
        );
        ctx.closePath();
        ctx.fill();
        break;
      }
      case 'pen': {
        ctx.strokeStyle = shape.color;
        ctx.lineWidth = shape.width;
        ctx.beginPath();
        shape.points.forEach((point, index) => {
          if (index === 0) ctx.moveTo(point.x, point.y);
          else ctx.lineTo(point.x, point.y);
        });
        ctx.stroke();
        break;
      }
      case 'mosaic': {
        drawMosaic(ctx, shape.rect);
        break;
      }
      case 'text': {
        ctx.fillStyle = shape.color;
        ctx.font = `${shape.size}px "Segoe UI", "Microsoft YaHei", sans-serif`;
        ctx.textBaseline = 'top';
        const firstLine = shape.size * 1.25;
        shape.text.split('\n').forEach((line, index) => {
          ctx.fillText(line, shape.at.x, shape.at.y + index * firstLine);
        });
        break;
      }
    }
  }

  /** 马赛克：从底图取区域缩到 1/14 再放大回来（禁用平滑 = 像素块）。 */
  function drawMosaic(ctx: CanvasRenderingContext2D, rawRect: Rect): void {
    if (!baseImage) return;
    const rect = norm(rawRect);
    const width = Math.max(1, Math.round(rect.x2 - rect.x1));
    const height = Math.max(1, Math.round(rect.y2 - rect.y1));
    const block = 14;
    const tinyW = Math.max(1, Math.floor(width / block));
    const tinyH = Math.max(1, Math.floor(height / block));
    const scratch = document.createElement('canvas');
    scratch.width = tinyW;
    scratch.height = tinyH;
    const tiny = scratch.getContext('2d');
    if (!tiny) return;
    tiny.drawImage(baseImage, rect.x1, rect.y1, width, height, 0, 0, tinyW, tinyH);
    ctx.imageSmoothingEnabled = false;
    ctx.drawImage(scratch, 0, 0, tinyW, tinyH, rect.x1, rect.y1, width, height);
    ctx.imageSmoothingEnabled = true;
  }

  // ---- 交互 ----
  /**
   * 统一优先级分发器（设计 §4.1）：
   * 文字草稿提交 > 选区手柄（任何工具）> 选区边带 ≤5dpr（任何工具）>
   * 无工具态：标注手柄 > 标注拖动 > 框内空白移动选区 >
   * 绘图工具态：文字落点 / 起笔绘制 > 选区外任何工具都重开选区。
   */
  function onMouseDown(event: MouseEvent): void {
    if (!canvas || !baseImage) return;
    if (event.button !== 0) return;
    if (textDraft) {
      commitText();
      return;
    }
    const point = toPhysical(event);

    if (phase === 'idle') {
      // 开新选区。悬停命中先记下：原地松开（未拖动）= 点击接受窗口选区。
      pressHover = hoverRect ? { ...norm(hoverRect) } : null;
      hoverRect = null;
      phase = 'selecting';
      selection = { x1: point.x, y1: point.y, x2: point.x, y2: point.y };
      drag = { kind: 'new', origin: { ...selection }, start: point };
      toolbarOffset = { x: 0, y: 0 };
      render();
      return;
    }

    // ---- selected 态：优先级分发 ----
    // 2) 选区 8 手柄（任何工具态；外层几何优先于标注手柄）。
    const handle = hitHandle(point);
    if (handle >= 0) {
      selected = null;
      drag = { kind: 'resize', handle, origin: { ...selection }, start: point };
      return;
    }
    // 3) 选区边带 ≤5dpr → 对应向 resize（任何工具态）。
    const edge = hitSelectionEdge(point);
    if (edge >= 0) {
      selected = null;
      drag = { kind: 'resize', handle: edge, origin: { ...selection }, start: point };
      return;
    }

    const sel = norm(selection);
    const inside = point.x >= sel.x1 && point.x <= sel.x2 && point.y >= sel.y1 && point.y <= sel.y2;
    if (!inside) {
      // 6) 选区外（任何工具）→ 清标注重开选区（微信行为）。
      tool = 'none';
      phase = 'selecting';
      selection = { x1: point.x, y1: point.y, x2: point.x, y2: point.y };
      drag = { kind: 'new', origin: { ...selection }, start: point };
      shapes = [];
      redoStack = [];
      selected = null;
      hoverRect = null;
      pressHover = null;
      toolbarOffset = { x: 0, y: 0 };
      return;
    }

    if (tool === 'none') {
      // 4a) 已选标注的手柄 → 缩放。
      if (selected !== null) {
        const current = shapes[selected];
        if (current && shapeResizable(current)) {
          const shapeHandle = hitBoxHandle(outlineBox(current), point);
          if (shapeHandle >= 0) {
            drag = {
              kind: 'shape-resize',
              index: selected,
              handle: shapeHandle,
              origin: cloneShape(current),
              start: point,
            };
            return;
          }
        }
      }
      // 4b) 标注命中 → 选中并拖动。
      const hit = hitShapeIndex(point);
      if (hit >= 0) {
        const shape = shapes[hit];
        if (shape) {
          selected = hit;
          drag = { kind: 'shape-move', index: hit, origin: cloneShape(shape), start: point };
          render();
        }
        return;
      }
      // 4c) 框内空白 → 拖动即移动选区。
      if (selected !== null) {
        selected = null;
        render();
      }
      drag = { kind: 'move', origin: { ...selection }, start: point };
      return;
    }

    // 5) 绘图工具态（框内）。
    if (tool === 'text') {
      selected = null;
      textDraft = { x: point.x, y: point.y, value: '' };
      return;
    }
    selected = null;
    liveShape = beginShape(tool, point, color, strokeWidth);
  }

  /**
   * 选区边带命中（设计 §4.1 第 3 级）：距任一边 ≤5dpr 的带状区域，
   * 返回对应方向的 resize 手柄号；角部同时命中两边 → 角手柄。
   */
  function hitSelectionEdge(point: Pt): number {
    const sel = norm(selection);
    const band = 5 * dpr;
    const nearLeft = Math.abs(point.x - sel.x1) <= band;
    const nearRight = Math.abs(point.x - sel.x2) <= band;
    const nearTop = Math.abs(point.y - sel.y1) <= band;
    const nearBottom = Math.abs(point.y - sel.y2) <= band;
    const withinY = point.y >= sel.y1 - band && point.y <= sel.y2 + band;
    const withinX = point.x >= sel.x1 - band && point.x <= sel.x2 + band;
    if (nearLeft && nearTop && withinY && withinX) return 0;
    if (nearRight && nearTop && withinY && withinX) return 2;
    if (nearLeft && nearBottom && withinY && withinX) return 5;
    if (nearRight && nearBottom && withinY && withinX) return 7;
    if (nearLeft && withinY) return 3;
    if (nearRight && withinY) return 4;
    if (nearTop && withinX) return 1;
    if (nearBottom && withinX) return 6;
    return -1;
  }

  function beginShape(
    active: Tool,
    point: { x: number; y: number },
    shapeColor: string,
    width: number,
  ): Shape | null {
    switch (active) {
      case 'rect':
      case 'mosaic':
        return {
          type: active,
          rect: { x1: point.x, y1: point.y, x2: point.x, y2: point.y },
          color: shapeColor,
          width,
        };
      case 'ellipse':
        return {
          type: 'ellipse',
          rect: { x1: point.x, y1: point.y, x2: point.x, y2: point.y },
          color: shapeColor,
          width,
        };
      case 'arrow':
        return { type: 'arrow', from: { ...point }, to: { ...point }, color: shapeColor, width };
      case 'pen':
        return { type: 'pen', points: [{ ...point }], color: shapeColor, width };
      default:
        return null;
    }
  }

  /** 工具栏拖动：仅把手/容器空白区按下生效（按钮与下拉上的按下照常工作）。 */
  function onToolbarMouseDown(event: MouseEvent): void {
    const target = event.target as HTMLElement | null;
    if (!target) return;
    const onGrip = target.closest('[data-toolbar-grip]') !== null;
    if (!onGrip && target !== event.currentTarget) return;
    event.preventDefault();
    closeMenus();
    toolbarDrag = {
      sx: event.clientX,
      sy: event.clientY,
      ox: toolbarOffset.x,
      oy: toolbarOffset.y,
    };
  }

  function onMouseMove(event: MouseEvent): void {
    if (!canvas) return;
    // 工具栏拖动：跟随指针，位置钳制在屏内（offset 记的是相对默认位的净偏移）。
    if (toolbarDrag) {
      const base = toolbarBase;
      if (base) {
        const cssW = canvas.clientWidth;
        const cssH = canvas.clientHeight;
        const x = Math.min(
          Math.max(base.x + toolbarDrag.ox + (event.clientX - toolbarDrag.sx), 8),
          Math.max(8, cssW - base.w - 8),
        );
        const y = Math.min(
          Math.max(base.y + toolbarDrag.oy + (event.clientY - toolbarDrag.sy), 8),
          Math.max(8, cssH - base.h - 8),
        );
        toolbarOffset = { x: x - base.x, y: y - base.y };
      }
      return;
    }
    const point = toPhysical(event);
    if (!drag && !liveShape) {
      updateCursor(point);
      if (phase === 'idle') scheduleHoverProbe(point);
      else if (hoverRect) {
        hoverRect = null;
        render();
      }
      return;
    }
    if (drag?.kind === 'new' || (drag?.kind === 'resize' && phase === 'selecting')) {
      selection = { ...selection, x2: point.x, y2: point.y };
    } else if (drag?.kind === 'move') {
      const deltaX = point.x - drag.start.x;
      const deltaY = point.y - drag.start.y;
      selection = {
        x1: drag.origin.x1 + deltaX,
        y1: drag.origin.y1 + deltaY,
        x2: drag.origin.x2 + deltaX,
        y2: drag.origin.y2 + deltaY,
      };
    } else if (drag?.kind === 'resize') {
      selection = resizeTo(drag.origin, drag.handle!, point);
    } else if (drag?.kind === 'shape-move') {
      shapes[drag.index] = translateShape(
        drag.origin,
        point.x - drag.start.x,
        point.y - drag.start.y,
      );
    } else if (drag?.kind === 'shape-resize') {
      shapes[drag.index] = resizeShape(drag.origin as RectShape, drag.handle, point);
    }
    if (liveShape) {
      if (
        liveShape.type === 'rect' ||
        liveShape.type === 'ellipse' ||
        liveShape.type === 'mosaic'
      ) {
        liveShape.rect = { ...liveShape.rect, x2: point.x, y2: point.y };
      } else if (liveShape.type === 'arrow') {
        liveShape.to = { ...point };
      } else if (liveShape.type === 'pen') {
        liveShape.points.push({ ...point });
      }
    }
    render();
  }

  /** 微信式窗口感知：节流探测（60ms + 位移 ≥3 物理像素），命中换算屏内坐标。 */
  function scheduleHoverProbe(point: Pt): void {
    const now = performance.now();
    const moved =
      Math.abs(point.x - hoverProbePoint.x) + Math.abs(point.y - hoverProbePoint.y) >= 3;
    if (!moved && hoverRect !== null) return;
    if (moved && now - hoverProbeAt < 60) return;
    hoverProbeAt = now;
    hoverProbePoint = { ...point };
    const virtualX = monitorX + point.x;
    const virtualY = monitorY + point.y;
    void call<WindowHit | null>('shots_window_at', { x: virtualX, y: virtualY })
      .then((hit) => {
        // 探测是异步的：回来时可能已进入选区态（按下开拖），丢弃过期结果。
        if (phase !== 'idle' || drag || !canvas) return;
        const next = hit
          ? {
              x1: hit.x - monitorX,
              y1: hit.y - monitorY,
              x2: hit.x - monitorX + hit.width,
              y2: hit.y - monitorY + hit.height,
            }
          : null;
        const changed =
          (next === null) !== (hoverRect === null) ||
          (next &&
            hoverRect &&
            (next.x1 !== hoverRect.x1 ||
              next.y1 !== hoverRect.y1 ||
              next.x2 !== hoverRect.x2 ||
              next.y2 !== hoverRect.y2));
        if (!changed) return;
        hoverRect = next;
        render();
      })
      .catch(() => {
        // 感知失败静默：自由框选始终可用。
      });
  }

  function resizeTo(origin: Rect, handle: number, point: { x: number; y: number }): Rect {
    const next = { ...origin };
    // 左列（handle 0/3/5）动 x1；右列（2/4/7）动 x2；上行（0/1/2）动 y1；下行（5/6/7）动 y2。
    if (handle === 0 || handle === 3 || handle === 5) next.x1 = point.x;
    if (handle === 2 || handle === 4 || handle === 7) next.x2 = point.x;
    if (handle === 0 || handle === 1 || handle === 2) next.y1 = point.y;
    if (handle === 5 || handle === 6 || handle === 7) next.y2 = point.y;
    return next;
  }

  // ---- 标注编辑（选中 / 拖动 / 缩放 / 删除）----

  type RectShape = Extract<Shape, { rect: Rect }>;

  /** 只有画框类形状支持缩放；箭头 / 画笔 / 文字只支持拖动。 */
  function shapeResizable(shape: Shape): shape is RectShape {
    return shape.type === 'rect' || shape.type === 'ellipse' || shape.type === 'mosaic';
  }

  function shapeBBox(shape: Shape): Rect {
    switch (shape.type) {
      case 'rect':
      case 'ellipse':
      case 'mosaic':
        return norm(shape.rect);
      case 'arrow':
        return norm({ x1: shape.from.x, y1: shape.from.y, x2: shape.to.x, y2: shape.to.y });
      case 'pen': {
        const xs = shape.points.map((p) => p.x);
        const ys = shape.points.map((p) => p.y);
        return {
          x1: Math.min(...xs),
          y1: Math.min(...ys),
          x2: Math.max(...xs),
          y2: Math.max(...ys),
        };
      }
      case 'text': {
        const lines = shape.text.split('\n');
        const context = canvas?.getContext('2d');
        let width = shape.size * Math.max(...lines.map((line) => line.length));
        if (context) {
          context.font = `${shape.size}px "Segoe UI", "Microsoft YaHei", sans-serif`;
          width = Math.max(...lines.map((line) => context.measureText(line).width));
        }
        return {
          x1: shape.at.x,
          y1: shape.at.y,
          x2: shape.at.x + width,
          y2: shape.at.y + lines.length * shape.size * 1.25,
        };
      }
    }
  }

  /** 编辑框 = 包围盒外扩（描边宽 + 余量）：虚线画在框上，手柄也布在框上。 */
  function outlineBox(shape: Shape): Rect {
    const box = shapeBBox(shape);
    const pad = ('width' in shape ? shape.width / 2 : 2 * dpr) + 3 * dpr;
    return { x1: box.x1 - pad, y1: box.y1 - pad, x2: box.x2 + pad, y2: box.y2 + pad };
  }

  function cloneShape(shape: Shape): Shape {
    return JSON.parse(JSON.stringify(shape)) as Shape;
  }

  function distToSegment(point: Pt, from: Pt, to: Pt): number {
    const deltaX = to.x - from.x;
    const deltaY = to.y - from.y;
    const lengthSq = deltaX * deltaX + deltaY * deltaY;
    const t =
      lengthSq === 0 ? 0 : ((point.x - from.x) * deltaX + (point.y - from.y) * deltaY) / lengthSq;
    const clamped = Math.max(0, Math.min(1, t));
    return Math.hypot(point.x - (from.x + clamped * deltaX), point.y - (from.y + clamped * deltaY));
  }

  function hitShape(shape: Shape, point: Pt): boolean {
    const slack = 6 * dpr;
    if (shape.type === 'arrow') {
      return distToSegment(point, shape.from, shape.to) <= shape.width / 2 + slack;
    }
    if (shape.type === 'pen') {
      const tolerance = shape.width / 2 + slack;
      for (let index = 1; index < shape.points.length; index++) {
        const from = shape.points[index - 1];
        const to = shape.points[index];
        if (from && to && distToSegment(point, from, to) <= tolerance) return true;
      }
      return false;
    }
    const box = outlineBox(shape);
    return point.x >= box.x1 && point.x <= box.x2 && point.y >= box.y1 && point.y <= box.y2;
  }

  /** 自上而下找第一个命中的标注。 */
  function hitShapeIndex(point: Pt): number {
    for (let index = shapes.length - 1; index >= 0; index--) {
      const shape = shapes[index];
      if (shape && hitShape(shape, point)) return index;
    }
    return -1;
  }

  function translateShape(shape: Shape, deltaX: number, deltaY: number): Shape {
    switch (shape.type) {
      case 'rect':
      case 'ellipse':
      case 'mosaic':
        return {
          ...shape,
          rect: {
            x1: shape.rect.x1 + deltaX,
            y1: shape.rect.y1 + deltaY,
            x2: shape.rect.x2 + deltaX,
            y2: shape.rect.y2 + deltaY,
          },
        };
      case 'arrow':
        return {
          ...shape,
          from: { x: shape.from.x + deltaX, y: shape.from.y + deltaY },
          to: { x: shape.to.x + deltaX, y: shape.to.y + deltaY },
        };
      case 'pen':
        return {
          ...shape,
          points: shape.points.map((p) => ({ x: p.x + deltaX, y: p.y + deltaY })),
        };
      case 'text':
        return { ...shape, at: { x: shape.at.x + deltaX, y: shape.at.y + deltaY } };
    }
  }

  /** 框类形状缩放：手柄动对应边，另一侧为锚；限制最小尺寸。 */
  function resizeShape(shape: RectShape, handle: number, point: Pt): RectShape {
    const minSize = 8 * dpr;
    const rect = resizeTo(norm(shape.rect), handle, point);
    if (rect.x2 - rect.x1 < minSize) {
      if (handle === 0 || handle === 3 || handle === 5) rect.x1 = rect.x2 - minSize;
      else rect.x2 = rect.x1 + minSize;
    }
    if (rect.y2 - rect.y1 < minSize) {
      if (handle === 0 || handle === 1 || handle === 2) rect.y1 = rect.y2 - minSize;
      else rect.y2 = rect.y1 + minSize;
    }
    return { ...shape, rect };
  }

  function onMouseUp(): void {
    toolbarDrag = null;
    if (drag && (drag.kind === 'shape-move' || drag.kind === 'shape-resize')) {
      // 标注编辑是一次新改动，使重做历史失效。
      redoStack = [];
      drag = null;
      render();
      return;
    }
    if (drag) {
      const sel = norm(selection);
      const tooSmall = sel.x2 - sel.x1 < 8 || sel.y2 - sel.y1 < 8;
      if (drag.kind === 'new' && tooSmall) {
        if (pressHover) {
          // 原地点击（未拖动）+ 悬停命中 → 微信行为：接受窗口选区。
          phase = 'selected';
          selection = pressHover;
        } else {
          phase = 'idle';
          selection = { x1: 0, y1: 0, x2: 0, y2: 0 };
        }
      } else {
        phase = 'selected';
        selection = sel;
      }
      pressHover = null;
      drag = null;
    }
    if (liveShape) {
      // 空形状不进栈（如零距离箭头）。
      let empty = false;
      if (liveShape.type === 'arrow') {
        empty =
          Math.hypot(liveShape.to.x - liveShape.from.x, liveShape.to.y - liveShape.from.y) < 6;
      } else if (liveShape.type !== 'pen') {
        const rect = norm((liveShape as { rect: Rect }).rect);
        empty = rect.x2 - rect.x1 < 4 && rect.y2 - rect.y1 < 4;
      } else if (liveShape.points.length < 2) {
        empty = true;
      }
      if (!empty) {
        shapes = [...shapes, liveShape];
        redoStack = [];
      }
      liveShape = null;
    }
    render();
  }

  /** 8 手柄的标准布点（顺时针自左上），裁剪选区与形状编辑框共用。 */
  function boxHandlePositions(box: Rect): Array<[number, number]> {
    return [
      [box.x1, box.y1],
      [(box.x1 + box.x2) / 2, box.y1],
      [box.x2, box.y1],
      [box.x1, (box.y1 + box.y2) / 2],
      [box.x2, (box.y1 + box.y2) / 2],
      [box.x1, box.y2],
      [(box.x1 + box.x2) / 2, box.y2],
      [box.x2, box.y2],
    ];
  }

  function hitBoxHandle(box: Rect, point: { x: number; y: number }): number {
    const radius = 7 * dpr;
    const positions = boxHandlePositions(box);
    for (let index = 0; index < positions.length; index++) {
      const position = positions[index];
      if (!position) continue;
      const [x, y] = position;
      if (Math.abs(point.x - x) <= radius && Math.abs(point.y - y) <= radius) return index;
    }
    return -1;
  }

  function hitHandle(point: { x: number; y: number }): number {
    return hitBoxHandle(norm(selection), point);
  }

  const HANDLE_CURSORS = [
    'nwse-resize',
    'ns-resize',
    'nesw-resize',
    'ew-resize',
    'ew-resize',
    'nesw-resize',
    'ns-resize',
    'nwse-resize',
  ];

  function updateCursor(point: { x: number; y: number }): void {
    if (!canvas) return;
    let cursor = 'crosshair';
    if (phase === 'selected') {
      // 边线/手柄最优先（任何工具态）：选区手柄 → 选区边带。
      const selectionHandle = hitHandle(point);
      const edge = selectionHandle >= 0 ? selectionHandle : hitSelectionEdge(point);
      if (edge >= 0) {
        cursor = HANDLE_CURSORS[edge] ?? 'crosshair';
      } else {
        const sel = norm(selection);
        const inside =
          point.x >= sel.x1 && point.x <= sel.x2 && point.y >= sel.y1 && point.y <= sel.y2;
        if (inside) {
          if (tool === 'none') {
            // 已选标注的手柄提示缩放方向；其余框内区域一律可拖动。
            const current = selected !== null ? shapes[selected] : undefined;
            const shapeHandle =
              current && shapeResizable(current) ? hitBoxHandle(outlineBox(current), point) : -1;
            cursor = shapeHandle >= 0 ? (HANDLE_CURSORS[shapeHandle] ?? 'move') : 'move';
          } else if (tool === 'text') {
            cursor = 'text';
          } else {
            cursor = 'crosshair';
          }
        }
      }
    }
    canvas.style.cursor = cursor;
  }

  function onDoubleClick(event: MouseEvent): void {
    if (phase !== 'selected' || textDraft) return;
    // 双击复制仅在无工具态生效（设计 §4.2）。
    if (tool !== 'none') return;
    // 双击标注是编辑意图，不触发"双击复制"。
    if (hitShapeIndex(toPhysical(event)) >= 0) return;
    void produce('copy');
  }

  function commitText(): void {
    if (!textDraft) return;
    const value = textDraft.value.trim();
    if (value) {
      shapes = [
        ...shapes,
        {
          type: 'text',
          at: { x: textDraft.x, y: textDraft.y },
          text: textDraft.value,
          color,
          size: Math.round(20 * dpr),
        },
      ];
      redoStack = [];
    }
    textDraft = null;
    render();
  }

  /** 文字输入框的键盘处理：Enter 落墨，Esc 丢弃。 */
  function onTextKeydown(event: KeyboardEvent): void {
    if (event.key === 'Enter') {
      event.preventDefault();
      commitText();
    } else if (event.key === 'Escape') {
      event.preventDefault();
      textDraft = null;
    }
  }

  function undo(): void {
    const last = shapes[shapes.length - 1];
    if (!last) return;
    redoStack = [...redoStack, last];
    shapes = shapes.slice(0, -1);
    // 选中的标注被撤销掉时清空选中。
    if (selected !== null && selected >= shapes.length) selected = null;
    render();
  }

  function redo(): void {
    const next = redoStack[redoStack.length - 1];
    if (!next) return;
    shapes = [...shapes, next];
    redoStack = redoStack.slice(0, -1);
    render();
  }

  // ---- 产出 ----
  async function produce(kind: 'copy' | 'save' | 'pin'): Promise<void> {
    if (working) return;
    working = true;
    message = '';
    try {
      const png = exportSelectionPng();
      const base64 = png.slice(png.indexOf(',') + 1);
      if (kind === 'copy') {
        await call('shots_copy_clipboard', { pngBase64: base64 });
        message = '已复制到剪贴板';
      } else if (kind === 'save') {
        const saved = await call<string>('shots_save', { pngBase64: base64 });
        message = `已保存：${saved}`;
      } else {
        const path = await call<string>('shots_pin', { pngBase64: base64 });
        await call('shots_open_pin', { path });
      }
      await exitAll();
    } catch (error) {
      message = error instanceof Error ? error.message : String(error);
      working = false;
    }
  }

  function exportSelectionPng(): string {
    const sel = norm(selection);
    const width = Math.max(1, Math.round(sel.x2 - sel.x1));
    const height = Math.max(1, Math.round(sel.y2 - sel.y1));
    const out = document.createElement('canvas');
    out.width = width;
    out.height = height;
    const ctx = out.getContext('2d');
    if (!ctx || !baseImage) throw new Error('导出画布不可用');
    ctx.drawImage(baseImage, sel.x1, sel.y1, width, height, 0, 0, width, height);
    if (shapes.length > 0) {
      ctx.lineCap = 'round';
      ctx.lineJoin = 'round';
      ctx.translate(-sel.x1, -sel.y1);
      for (const shape of shapes) drawShape(ctx, shape);
    }
    return out.toDataURL('image/png');
  }

  async function exitAll(): Promise<void> {
    await call('shots_close_overlays');
  }
</script>

<svelte:window
  onmousedown={() => closeMenus()}
  onmousemove={onMouseMove}
  onmouseup={onMouseUp}
  ondblclick={onDoubleClick}
/>

<div class="fixed inset-0 select-none overflow-hidden bg-black">
  <canvas
    bind:this={canvas}
    width={baseImage?.naturalWidth ?? 0}
    height={baseImage?.naturalHeight ?? 0}
    class="block h-full w-full"
    onmousedown={onMouseDown}
  ></canvas>

  {#if phase !== 'idle' && !textDraft}
    {@const sel = norm(selection)}
    {@const selW = (sel.x2 - sel.x1) / dpr}
    {@const selH = (sel.y2 - sel.y1) / dpr}
    {@const inside = selW >= 90 && selH >= 30}
    <!-- 尺寸提示：默认放选区内左上角（全屏截图也可见）；选区太小装不下时
         回退到选区下方（贴底边时改上方）。拖选过程中同样跟随。 -->
    <div
      class="pointer-events-none absolute rounded bg-black/70 px-2 py-0.5 text-xs text-white"
      style="left: {inside ? sel.x1 / dpr + 6 : (sel.x1 + sel.x2) / 2 / dpr}px;
        top: {inside
        ? sel.y1 / dpr + 6
        : sel.y2 / dpr + 6 + 20 < (canvas?.clientHeight ?? 0)
          ? sel.y2 / dpr + 6
          : sel.y1 / dpr - 26}px;
        {inside ? '' : 'transform: translateX(-50%);'}"
    >
      {sizeLabel}
    </div>
  {/if}

  {#if textDraft}
    <!-- 文字输入：绝对定位（物理→CSS） -->
    <input
      class="absolute z-10 min-w-16 border-none bg-transparent p-0 leading-tight outline-none"
      style="left: {textDraft.x / dpr}px; top: {textDraft.y /
        dpr}px; color: {color}; font-size: {20 *
        dpr}px; font-family: 'Segoe UI', 'Microsoft YaHei', sans-serif;"
      type="text"
      placeholder="输入文字"
      value={textDraft.value}
      oninput={(event) => {
        if (!textDraft) return;
        textDraft = { x: textDraft.x, y: textDraft.y, value: event.currentTarget.value };
      }}
      onkeydown={onTextKeydown}
    />
  {/if}

  {#if toolbarBox}
    <div
      bind:clientWidth={toolbarW}
      role="toolbar"
      aria-label="标注工具"
      tabindex="-1"
      class="absolute z-10 flex items-center gap-1 rounded-lg border border-white/15 bg-neutral-900/95 px-1.5 py-1.5 shadow-xl"
      style="left: {toolbarBox.x}px; top: {toolbarBox.y}px;"
      onmousedown={onToolbarMouseDown}
    >
      <!-- 拖动把手：把手上（或容器空白处）按下即可拖走工具栏 -->
      <span
        data-toolbar-grip
        class="mr-0.5 flex size-6 cursor-grab items-center justify-center rounded text-neutral-500 transition-colors hover:text-neutral-300 active:cursor-grabbing"
        title="拖动工具栏"
      >
        <svg viewBox="0 0 24 24" class="size-3.5" fill="currentColor">
          <circle cx="9" cy="6" r="1.7"></circle>
          <circle cx="15" cy="6" r="1.7"></circle>
          <circle cx="9" cy="12" r="1.7"></circle>
          <circle cx="15" cy="12" r="1.7"></circle>
          <circle cx="9" cy="18" r="1.7"></circle>
          <circle cx="15" cy="18" r="1.7"></circle>
        </svg>
      </span>

      {#each TOOLS as item (item.id)}
        <button
          class="flex size-8 items-center justify-center rounded transition-colors {tool === item.id
            ? 'bg-sky-500 text-white'
            : 'text-neutral-300 hover:bg-white/10'}"
          title={item.label}
          onclick={() => {
            // 再点激活中的工具 = 切回无工具态（拖选区/编辑标注）。
            tool = tool === item.id ? 'none' : item.id;
            if (tool !== 'text') textDraft = null;
            // 切到绘图工具即退出标注编辑态。
            if (tool !== 'none') selected = null;
          }}
        >
          <svg
            viewBox="0 0 24 24"
            class="size-4"
            fill="none"
            stroke="currentColor"
            stroke-width="1.6"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d={item.icon}></path>
          </svg>
        </button>
      {/each}

      <span class="mx-1 h-5 w-px bg-white/15"></span>

      <!-- 颜色：按钮只显示当前选中，点开下拉挑选（紧凑） -->
      <div class="relative">
        <button
          class="flex h-8 items-center gap-0.5 rounded px-1.5 transition-colors hover:bg-white/10"
          title="颜色 {color}"
          aria-label="选择颜色，当前 {color}"
          onmousedown={(event) => {
            event.stopPropagation();
            widthOpen = false;
            colorOpen = !colorOpen;
          }}
        >
          <span class="size-4 rounded-full border border-white/60" style="background: {color};"
          ></span>
          <svg
            viewBox="0 0 24 24"
            class="size-3 text-neutral-400"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="m6 9 6 6 6-6"></path>
          </svg>
        </button>
        {#if colorOpen}
          <div
            role="menu"
            aria-label="颜色"
            tabindex="-1"
            class="absolute left-1/2 z-20 flex -translate-x-1/2 items-center gap-1.5 rounded-lg border border-white/15 bg-neutral-900/95 px-2.5 py-2 shadow-xl {toolbarBox.y >
            120
              ? 'bottom-full mb-2'
              : 'top-full mt-2'}"
            onmousedown={(event) => event.stopPropagation()}
          >
            {#each COLORS as swatch (swatch)}
              <button
                role="menuitemradio"
                aria-checked={color === swatch}
                class="size-5 rounded-full border {color === swatch
                  ? 'border-white ring-2 ring-white/40'
                  : 'border-white/30'}"
                style="background: {swatch};"
                title={swatch}
                aria-label="颜色 {swatch}"
                onmousedown={(event) => {
                  event.stopPropagation();
                  color = swatch;
                  colorOpen = false;
                }}
              ></button>
            {/each}
          </div>
        {/if}
      </div>

      <!-- 线宽：按钮只显示当前粗细，点开下拉挑选（紧凑） -->
      <div class="relative">
        <button
          class="flex h-8 items-center gap-0.5 rounded px-1.5 transition-colors hover:bg-white/10"
          title="线宽 {strokeWidth / dpr}"
          aria-label="选择线宽，当前 {strokeWidth / dpr}"
          onmousedown={(event) => {
            event.stopPropagation();
            colorOpen = false;
            widthOpen = !widthOpen;
          }}
        >
          <span class="flex h-4 w-5 items-center justify-center">
            <span
              class="rounded-full bg-neutral-200"
              style="width: {(strokeWidth / dpr) * 2}px; height: {strokeWidth / dpr}px;"
            ></span>
          </span>
          <svg
            viewBox="0 0 24 24"
            class="size-3 text-neutral-400"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="m6 9 6 6 6-6"></path>
          </svg>
        </button>
        {#if widthOpen}
          <div
            role="menu"
            aria-label="线宽"
            tabindex="-1"
            class="absolute left-1/2 z-20 flex -translate-x-1/2 flex-col gap-0.5 rounded-lg border border-white/15 bg-neutral-900/95 p-1.5 shadow-xl {toolbarBox.y >
            120
              ? 'bottom-full mb-2'
              : 'top-full mt-2'}"
            onmousedown={(event) => event.stopPropagation()}
          >
            {#each WIDTHS as size (size)}
              <button
                role="menuitemradio"
                aria-checked={strokeWidth === size * dpr}
                class="flex h-8 w-20 items-center justify-center gap-2 rounded transition-colors {strokeWidth ===
                size * dpr
                  ? 'bg-sky-500'
                  : 'hover:bg-white/10'}"
                title="线宽 {size}"
                aria-label="线宽 {size}"
                onmousedown={(event) => {
                  event.stopPropagation();
                  strokeWidth = size * dpr;
                  widthOpen = false;
                }}
              >
                <span
                  class="rounded-full bg-neutral-200"
                  style="width: {size * 2.5}px; height: {size}px;"
                ></span>
                <span class="w-6 text-left text-xs text-neutral-300"
                  >{size === 2 ? '细' : size === 4 ? '中' : '粗'}</span
                >
              </button>
            {/each}
          </div>
        {/if}
      </div>

      <span class="mx-1 h-5 w-px bg-white/15"></span>

      <button
        class="flex size-8 items-center justify-center rounded text-neutral-300 transition-colors hover:bg-white/10 disabled:opacity-40"
        title="撤销"
        aria-label="撤销"
        disabled={shapes.length === 0}
        onclick={undo}
      >
        <svg
          viewBox="0 0 24 24"
          class="size-4"
          fill="none"
          stroke="currentColor"
          stroke-width="1.6"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <path d="M9 14 4 9l5-5"></path>
          <path d="M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5 5.5 5.5 0 0 1-5.5 5.5H11"></path>
        </svg>
      </button>
      <button
        class="flex size-8 items-center justify-center rounded text-neutral-300 transition-colors hover:bg-white/10 disabled:opacity-40"
        title="重做"
        aria-label="重做"
        disabled={redoStack.length === 0}
        onclick={redo}
      >
        <svg
          viewBox="0 0 24 24"
          class="size-4"
          fill="none"
          stroke="currentColor"
          stroke-width="1.6"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <path d="m15 14 5-5-5-5"></path>
          <path d="M20 9H9.5A5.5 5.5 0 0 0 4 14.5 5.5 5.5 0 0 0 9.5 20H13"></path>
        </svg>
      </button>

      <span class="mx-1 h-5 w-px bg-white/15"></span>

      <button
        class="rounded bg-sky-500 px-2.5 py-1 text-xs font-medium text-white hover:bg-sky-400 disabled:opacity-50"
        disabled={working}
        onclick={() => void produce('copy')}>复制</button
      >
      <button
        class="rounded bg-white/10 px-2.5 py-1 text-xs text-neutral-200 hover:bg-white/20 disabled:opacity-50"
        disabled={working}
        onclick={() => void produce('save')}>保存</button
      >
      <button
        class="rounded bg-white/10 px-2.5 py-1 text-xs text-neutral-200 hover:bg-white/20 disabled:opacity-50"
        disabled={working}
        onclick={() => void produce('pin')}>贴图</button
      >
      <button
        class="rounded px-2 py-1 text-xs text-neutral-400 hover:bg-white/10"
        onclick={() => void exitAll()}>退出</button
      >
    </div>
  {/if}

  {#if message}
    <div
      class="absolute bottom-6 left-1/2 -translate-x-1/2 rounded bg-black/80 px-3 py-1.5 text-xs text-white"
    >
      {message}
    </div>
  {/if}
</div>
