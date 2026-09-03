<script lang="ts">
  /**
   * 截屏覆盖窗（#/overlay?monitor=N&path=…）：微信式交互。
   * 冻结帧做底图 → 暗化遮罩 + 十字光标 → 拖框选区（可移动/8 手柄微调）
   * → 标注工具栏（矩形/椭圆/箭头/画笔/马赛克/文字 + 撤销重做）
   * → 产出（复制/保存/贴图/退出）。双击选区 = 复制；Esc = 退出。
   */
  import { onMount, tick } from 'svelte';
  import { convertFileSrc } from '@tauri-apps/api/core';
  import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
  import { call } from '../../lib/ipc';

  // ---- URL 参数 ----
  const params = new URLSearchParams(window.location.hash.split('?')[1] ?? '');
  const imagePath = params.get('path') ?? '';

  // ---- DOM ----
  let canvas: HTMLCanvasElement | null = $state(null);
  let baseImage = $state<HTMLImageElement | null>(null);

  // ---- 坐标系：全部物理像素（canvas.width = 图像物理宽）----
  let dpr = $state(1);

  type Tool = 'none' | 'rect' | 'ellipse' | 'arrow' | 'pen' | 'mosaic' | 'text';
  type Rect = { x1: number; y1: number; x2: number; y2: number };

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
  let textDraft = $state<{ x: number; y: number; value: string } | null>(null);

  const TOOLS: Array<{ id: Tool; label: string; icon: string }> = [
    { id: 'none', label: '选择', icon: 'M5 3l14 9-6 1 4 6-3 2-4-6-4 5z' },
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
  const toolbarBox = $derived.by(() => {
    if (phase !== 'selected') return null;
    const phys = norm(selection);
    const w = 430;
    const h = 44;
    let x = phys.x1 / dpr;
    let y = phys.y2 / dpr + 12;
    const cssW = canvas?.clientWidth ?? 0;
    const cssH = canvas?.clientHeight ?? 0;
    if (y + h > cssH - 8) y = phys.y1 / dpr - h - 12;
    if (y < 8) y = 8;
    x = Math.min(Math.max(x, 8), cssW - w - 8);
    return { x, y, w, h };
  });

  const sizeLabel = $derived.by(() => {
    if (phase !== 'selected') return '';
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
        // 先取消标注选中，再退出截屏。
        if (tool === 'none' && selected !== null) {
          selected = null;
          render();
          return;
        }
        void exitAll();
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

    // 暗化遮罩：选区外（未选区时全屏）。
    const hasSelection = phase !== 'idle';
    const sel = hasSelection ? norm(selection) : null;
    ctx.save();
    ctx.fillStyle = 'rgba(0, 0, 0, 0.45)';
    ctx.beginPath();
    ctx.rect(0, 0, canvas.width, canvas.height);
    if (sel) ctx.rect(sel.x1, sel.y1, sel.x2 - sel.x1, sel.y2 - sel.y1);
    ctx.fill('evenodd');
    ctx.restore();

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
  function onMouseDown(event: MouseEvent): void {
    if (!canvas || !baseImage) return;
    if (event.button !== 0) return;
    if (textDraft) {
      commitText();
      return;
    }
    const point = toPhysical(event);
    if (phase === 'idle' || tool !== 'none') {
      if (tool === 'text') {
        const sel = norm(selection);
        if (
          phase === 'selected' &&
          point.x >= sel.x1 &&
          point.x <= sel.x2 &&
          point.y >= sel.y1 &&
          point.y <= sel.y2
        ) {
          selected = null;
          textDraft = { x: point.x, y: point.y, value: '' };
        }
        return;
      }
      if (tool === 'none' && phase === 'idle') {
        // 开新选区。
        phase = 'selecting';
        selection = { x1: point.x, y1: point.y, x2: point.x, y2: point.y };
        drag = { kind: 'new', origin: { ...selection }, start: point };
        return;
      }
      if (phase === 'selected') {
        // 标注工具在选区内起笔。
        const sel = norm(selection);
        const inside =
          point.x >= sel.x1 && point.x <= sel.x2 && point.y >= sel.y1 && point.y <= sel.y2;
        if (!inside) {
          // 点到选区外：重开选区（微信行为）。
          tool = 'none';
          phase = 'selecting';
          selection = { x1: point.x, y1: point.y, x2: point.x, y2: point.y };
          drag = { kind: 'new', origin: { ...selection }, start: point };
          shapes = [];
          redoStack = [];
          selected = null;
          return;
        }
        selected = null;
        liveShape = beginShape(tool, point, color, strokeWidth);
        return;
      }
      return;
    }
    if (phase === 'selected' && tool === 'none') {
      // 1) 已选中形状：优先试它自己的缩放手柄。
      if (selected !== null) {
        const current = shapes[selected];
        if (current && shapeResizable(current)) {
          const handle = hitBoxHandle(outlineBox(current), point);
          if (handle >= 0) {
            drag = {
              kind: 'shape-resize',
              index: selected,
              handle,
              origin: cloneShape(current),
              start: point,
            };
            return;
          }
        }
      }
      // 2) 点到任意标注：选中并进入拖动。
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
      // 3) 空白处：先取消选中，再按选区手柄 / 移动 / 重开处理。
      if (selected !== null) {
        selected = null;
        render();
      }
      const handle = hitHandle(point);
      if (handle >= 0) {
        drag = { kind: 'resize', handle, origin: { ...selection }, start: point };
        return;
      }
      const sel = norm(selection);
      if (point.x >= sel.x1 && point.x <= sel.x2 && point.y >= sel.y1 && point.y <= sel.y2) {
        drag = { kind: 'move', origin: { ...selection }, start: point };
        return;
      }
      // 选区外：重开。
      phase = 'selecting';
      selection = { x1: point.x, y1: point.y, x2: point.x, y2: point.y };
      shapes = [];
      redoStack = [];
      drag = { kind: 'new', origin: { ...selection }, start: point };
    }
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

  function onMouseMove(event: MouseEvent): void {
    if (!canvas) return;
    const point = toPhysical(event);
    if (!drag && !liveShape) {
      updateCursor(point);
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
        phase = 'idle';
        selection = { x1: 0, y1: 0, x2: 0, y2: 0 };
      } else {
        phase = 'selected';
        selection = sel;
      }
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
    if (phase === 'selected' && tool === 'none') {
      const current = selected !== null ? shapes[selected] : undefined;
      // 已选中形状的手柄优先提示缩放方向。
      const shapeHandle =
        current && shapeResizable(current) ? hitBoxHandle(outlineBox(current), point) : -1;
      const handle = shapeHandle >= 0 ? shapeHandle : hitHandle(point);
      if (handle >= 0) {
        cursor = HANDLE_CURSORS[handle] ?? 'crosshair';
      } else if (hitShapeIndex(point) >= 0) {
        // 悬停在标注上：提示可拖动。
        cursor = 'move';
      } else {
        const sel = norm(selection);
        const inside =
          point.x >= sel.x1 && point.x <= sel.x2 && point.y >= sel.y1 && point.y <= sel.y2;
        cursor = inside ? 'move' : 'crosshair';
      }
    }
    canvas.style.cursor = cursor;
  }

  function onDoubleClick(event: MouseEvent): void {
    if (phase !== 'selected' || textDraft) return;
    // 双击标注是编辑意图，不触发"双击复制"。
    if (tool === 'none' && hitShapeIndex(toPhysical(event)) >= 0) return;
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

<svelte:window onmousemove={onMouseMove} onmouseup={onMouseUp} ondblclick={onDoubleClick} />

<div class="fixed inset-0 select-none overflow-hidden bg-black">
  <canvas
    bind:this={canvas}
    width={baseImage?.naturalWidth ?? 0}
    height={baseImage?.naturalHeight ?? 0}
    class="block h-full w-full"
    onmousedown={onMouseDown}
  ></canvas>

  {#if phase === 'selected' && !textDraft}
    <!-- 尺寸提示：选区上方居中 -->
    <div
      class="pointer-events-none absolute rounded bg-black/70 px-2 py-0.5 text-xs text-white"
      style="left: {(norm(selection).x1 + norm(selection).x2) / 2 / dpr}px;
        top: {norm(selection).y1 / dpr - 26}px;
        transform: translateX(-50%);"
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
      class="absolute z-10 flex items-center gap-1 rounded-lg border border-white/15 bg-neutral-900/95 px-2 py-1.5 shadow-xl"
      style="left: {toolbarBox.x}px; top: {toolbarBox.y}px;"
    >
      {#each TOOLS as item (item.id)}
        <button
          class="flex size-8 items-center justify-center rounded transition-colors {tool === item.id
            ? 'bg-sky-500 text-white'
            : 'text-neutral-300 hover:bg-white/10'}"
          title={item.label}
          onclick={() => {
            tool = item.id;
            if (item.id !== 'text') textDraft = null;
            // 切到绘图工具即退出标注编辑态。
            if (item.id !== 'none') selected = null;
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

      {#each COLORS as swatch (swatch)}
        <button
          class="size-4 rounded-full border {color === swatch
            ? 'border-white ring-2 ring-white/40'
            : 'border-white/30'}"
          style="background: {swatch};"
          title={swatch}
          onclick={() => (color = swatch)}
        ></button>
      {/each}

      <span class="mx-1 h-5 w-px bg-white/15"></span>

      {#each WIDTHS as size (size)}
        <button
          class="flex h-8 w-7 items-center justify-center rounded transition-colors {strokeWidth ===
          size
            ? 'bg-sky-500'
            : 'hover:bg-white/10'}"
          title="线宽 {size}"
          onclick={() => (strokeWidth = size * dpr)}
        >
          <span class="rounded-full bg-neutral-200" style="width: {size * 2}px; height: {size}px;"
          ></span>
        </button>
      {/each}

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
