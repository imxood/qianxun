import QRCode from 'qrcode/lib/core/qrcode';

/**
 * 配对二维码 → 单条 SVG path。
 *
 * 不用位图：SVG 在任何缩放下都是硬边（扫码器友好），且省掉 base64 进包
 * （4 KB 的 PNG 曾把首包撑大 3 倍）。这里只取 qrcode 的核心编码器，不带
 * canvas/png 渲染器——控制台不产出图片。
 */
export function qrSvgPath(text: string): { path: string; viewBox: string } {
  const code = QRCode.create(text, { errorCorrectionLevel: 'M' });
  const size = code.modules.size;
  const data = code.modules.data;
  let path = '';
  for (let y = 0; y < size; y += 1) {
    let x = 0;
    while (x < size) {
      if (data[y * size + x] !== 1) {
        x += 1;
        continue;
      }
      // 同行连续的暗模块合并成一段 run：路径长度约为逐格写法的 1/3。
      let run = 1;
      while (x + run < size && data[y * size + x + run] === 1) run += 1;
      path += `M${x} ${y}h${run}v1h-${run}z`;
      x += run;
    }
  }
  // 四周留 2 模块静默区，扫码器需要它来定位。
  const span = size + 4;
  return { path, viewBox: `-2 -2 ${span} ${span}` };
}
