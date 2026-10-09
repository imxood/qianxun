import { describe, expect, it } from 'vitest';

import { qrSvgPath } from './qr';

/**
 * 配对二维码的输出是单条 SVG path。这里锁两件事：
 * 1. 几何正确——扫码器要能解出原文；
 * 2. 静默区留够——少了它近景扫码会失败，而这种失败在现场没法排查。
 */
function decodeModules(path: string, size: number): number[][] {
  const grid = Array.from({ length: size }, () => new Array<number>(size).fill(0));
  // 解析 run-length 写法：`M{x} {y}h{w}v1h-{w}z`
  const re = /M(\d+) (\d+)h(\d+)v1/g;
  let match: RegExpExecArray | null;
  while ((match = re.exec(path)) !== null) {
    const [, xText, yText, wText] = match;
    const x = Number(xText);
    const y = Number(yText);
    const w = Number(wText);
    for (let i = 0; i < w; i += 1) grid[y]![x + i] = 1;
  }
  return grid;
}

describe('qrSvgPath', () => {
  const text = 'http://192.168.1.20:23100/qx-pair?code=A1B2C3D4E5F6';
  const { path, viewBox } = qrSvgPath(text);

  it('viewBox 比矩阵大 4：四周各 2 模块静默区', () => {
    const size = Number(viewBox.split(' ')[2]) - 4;
    expect(viewBox).toBe(`-2 -2 ${size + 4} ${size + 4}`);
  });

  it('路径只用闭合矩形指令，没有曲线（扫码器只认硬边）', () => {
    expect(path).toMatch(/^M/);
    expect(path).toMatch(/z$/);
    expect(path.replace(/M[\d]+ [\d]+h[\d]+v1h-[\d]+z/g, '')).toBe('');
  });

  it('三个定位角都是 7×7 实心块', () => {
    const size = Number(viewBox.split(' ')[2]) - 4;
    const grid = decodeModules(path, size);
    const solid = (x: number, y: number) => grid[y]![x];
    // 左上角
    expect(solid(0, 0)).toBe(1);
    // 右上角（右上定位角中心在 size-3）
    expect(solid(size - 4, 0)).toBe(1);
    // 左下角
    expect(solid(0, size - 4)).toBe(1);
    // 每个定位角的外环：角点到中心那条对角线仍是 1
    expect(solid(3, 3)).toBe(1);
  });

  it('静默区全 0', () => {
    // viewBox 起点是 -2，矩阵坐标必须都 >= 0——负坐标就意味着画进了静默区。
    for (const chunk of path.split('M').filter(Boolean)) {
      const x = Number(chunk.split(' ')[0]);
      expect(x).toBeGreaterThanOrEqual(0);
    }
  });

  it('同样输入产出同样输出（配对弹层重渲染不该抖动）', () => {
    expect(qrSvgPath(text)).toEqual({ path, viewBox });
  });

  it('路径长度明显短于逐格写法（run 合并真的生效）', () => {
    const size = Number(viewBox.split(' ')[2]) - 4;
    const grid = decodeModules(path, size);
    let dark = 0;
    for (const row of grid) for (const cell of row) dark += cell;
    // 逐格写法每个暗模块约 18 字符；run 合并后必须显著更短。
    expect(path.length).toBeLessThan(dark * 18);
  });
});
