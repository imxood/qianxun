/**
 * Squarified treemap（Bruls et al.）：把一组面积铺进矩形，尽量接近正方形。
 * 纯函数：输入各块的相对面积与目标矩形，输出同序的矩形列表。
 */

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** 单行（沿短边排列的一组块）的最差长宽比。 */
function worst(row: number[], length: number): number {
  const sum = row.reduce((total, value) => total + value, 0);
  let max = 0;
  let min = Infinity;
  for (const value of row) {
    max = Math.max(max, value);
    min = Math.min(min, value);
  }
  const lengthSq = length * length;
  const sumSq = sum * sum;
  return Math.max((lengthSq * max) / sumSq, sumSq / (lengthSq * min));
}

/**
 * 面积 → 矩形。面积为 0 / 负的块会被跳过（不占位）。
 * 返回数组与输入同序（跳过的块给出零面积矩形，方便按下标取用）。
 */
export function squarify(areas: number[], rect: Rect): Rect[] {
  const result: Rect[] = areas.map(() => ({ x: rect.x, y: rect.y, w: 0, h: 0 }));
  const total = areas.reduce((sum, value) => sum + Math.max(0, value), 0);
  if (total <= 0 || rect.w <= 0 || rect.h <= 0) return result;

  const scale = (rect.w * rect.h) / total;
  // 只铺正面积块；记录原下标。
  const items = areas
    .map((value, index) => ({ area: Math.max(0, value) * scale, index }))
    .filter((item) => item.area > 0);

  let { x, y, w, h } = rect;
  let cursor = 0;
  while (cursor < items.length) {
    const length = Math.min(w, h);
    const row: Array<{ area: number; index: number }> = [items[cursor]!];
    let rowWorst = worst([row[0]!.area], length);
    let next = cursor + 1;
    while (next < items.length) {
      const candidate = row.map((item) => item.area);
      candidate.push(items[next]!.area);
      const nextWorst = worst(candidate, length);
      if (nextWorst <= rowWorst) {
        row.push(items[next]!);
        rowWorst = nextWorst;
        next += 1;
      } else {
        break;
      }
    }

    const sum = row.reduce((totalRow, item) => totalRow + item.area, 0);
    if (w >= h) {
      // 竖条贴左缘。
      const strip = sum / h;
      let offset = y;
      for (const item of row) {
        const height = item.area / h;
        result[item.index] = { x, y: offset, w: strip, h: height };
        offset += height;
      }
      x += strip;
      w -= strip;
    } else {
      // 横条贴上缘。
      const strip = sum / w;
      let offset = x;
      for (const item of row) {
        const width = item.area / w;
        result[item.index] = { x: offset, y, w: width, h: strip };
        offset += width;
      }
      y += strip;
      h -= strip;
    }
    cursor = next;
  }
  return result;
}
