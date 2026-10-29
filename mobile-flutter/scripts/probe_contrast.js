// 对比度探针（由 scripts/check_qx_contrast.py 注入执行）。
//
// 为什么**在页面里量**而不是静态算：
//   · `.qx-status-btn` 底色是半透明 `rgb(127 127 127 / .12)`，
//     真实对比度取决于它下面压着什么 —— 静态算只能拿底色去比。
//   · `.qx-hint-*` 挂在 DSH 自己的行里，背景是继承来的。
//
// 量法：取 computed color（前景）+ 沿祖先把半透明层**合成**出真实底色，
// 再按 WCAG 判阈值（大字 ≥24px 或 ≥18.66px 粗体 用 3:1，其余 4.5:1）。
(() => {
  const P = (c) => {
    const m = /rgba?\(([^)]+)\)/.exec(c);
    if (!m) return null;
    const p = m[1].split(',').map((x) => parseFloat(x));
    if (p.some((v) => Number.isNaN(v))) return null;
    return { r: p[0], g: p[1], b: p[2], a: p.length > 3 ? p[3] : 1 };
  };
  const LIN = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4); };
  const L = (c) => 0.2126 * LIN(c.r) + 0.7152 * LIN(c.g) + 0.0722 * LIN(c.b);
  const ratio = (a, b) => {
    const la = L(a); const lb = L(b);
    return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
  };
  const over = (fg, bg) => ({
    r: fg.r * fg.a + bg.r * (1 - fg.a),
    g: fg.g * fg.a + bg.g * (1 - fg.a),
    b: fg.b * fg.a + bg.b * (1 - fg.a),
    a: 1,
  });

  // 沿祖先往上合成底色；遇到第一个不透明的就停
  const effectiveBg = (el) => {
    let n = el;
    let acc = null;
    while (n && n !== document.documentElement) {
      const c = P(getComputedStyle(n).backgroundColor);
      if (c && c.a > 0) {
        acc = acc ? over(acc, c) : c;
        if (c.a >= 1) return acc;
      }
      n = n.parentElement;
    }
    const root = P(getComputedStyle(document.body).backgroundColor);
    return acc && root ? over(acc, root) : (root || { r: 255, g: 255, b: 255, a: 1 });
  };

  const out = {
    dark: matchMedia('(prefers-color-scheme: dark)').matches,
    items: [],
  };
  // ⚠️ **`.qx-hint-ctx` 已从列表里移除**（用户明确要求）：
  //    上下文占用现在只由 composer 工具栏的「N%」标签承担，
  //    列表里再放一份是重复信息，而且它与标题重叠 52px。
  //    所以它不再出现在目标清单里 —— 留着会让闸门永远报「未量到」。
  const targets = [
    ['状态药丸数值', '.qx-status-val'],
    ['稀疏行提示', '.qx-hint-sparse'],
    ['弹层标题', '.qx-sheet-title'],
    ['弹层关闭键', '.qx-sheet-x'],
    ['弹层正文', '.qx-sheet-body'],
    ['统计行文字', '.qx-stat-text'],
  ];
  for (const [name, sel] of targets) {
    const el = document.querySelector(sel);
    if (!el) { out.items.push({ name, found: false }); continue; }
    const cs = getComputedStyle(el);
    const fg = P(cs.color);
    const bg = effectiveBg(el);
    const r = el.getBoundingClientRect();
    const fs = parseFloat(cs.fontSize) || 0;
    const bold = parseInt(cs.fontWeight, 10) >= 600;
    const need = (fs >= 24 || (bold && fs >= 18.66)) ? 3 : 4.5;
    const cr = fg ? ratio(over(fg, bg), bg) : 0;
    out.items.push({
      name, found: true,
      color: cs.color,
      bg: 'rgb(' + [bg.r, bg.g, bg.b].map((v) => Math.round(v)).join(',') + ')',
      fontPx: fs, bold,
      ratio: Math.round(cr * 100) / 100,
      need, pass: cr >= need,
      visible: r.width > 1 && r.height > 1,
    });
  }
  return JSON.stringify(out);
})()