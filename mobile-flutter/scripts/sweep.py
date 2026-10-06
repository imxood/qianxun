"""巡检器：把每个页面/状态都走一遍，审查 + 截图，输出紧凑报告。

为什么要它
----------
迭代要覆盖「每一个页面 × 每一种状态」，手动点的话几十轮就会把上下文烧光。
所以：状态用脚本驱动、指标用脚本量，只有**画面好不好看**需要人眼，
而那部分我只挑最值得看的看。

它走的状态（全部通过 CDP 点 DOM 元素，不靠坐标）
--------------------------------------------------
  1 home          DSH 空态（hero + composer）
  2 workspaces    工作区独立页（☰ 打开）
  3 ws-expanded   某个工作区展开后的会话列表
  4 chat          Chat 页（有内容）
  5 trajectory    轨迹 tab
  6 search        搜索会话
  7 rightbar      右侧栏
  8 tool-open     工具卡展开

用法：
  python scripts/sweep.py                 # 全跑，输出报告 + 截图
  python scripts/sweep.py --shots         # 额外把截图列出路径
  python scripts/sweep.py --serial AEQC8P55MFCYBUFA
"""
from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CDP = ROOT / "scripts" / "cdp_webview.py"
SHOTS = ROOT / ".tmp" / "sweep"
SERIAL = ""
for i, a in enumerate(sys.argv):
    if a == "--serial" and i + 1 < len(sys.argv):
        SERIAL = sys.argv[i + 1]


def cdp(cmd: str, js: str | None = None, path: str | None = None) -> str:
    args = [sys.executable, str(CDP)]
    if SERIAL:
        args += ["--serial", SERIAL]
    if js is not None:
        f = ROOT / ".tmp" / "_sweep_expr.js"
        f.write_text(js, encoding="utf-8")
        args += ["eval", "--file", str(f)]
    elif path:
        args += ["shot", str(path)]
    else:
        args += [cmd]
    proc = subprocess.run(args, capture_output=True, text=True,
                          encoding="utf-8", errors="replace", timeout=300)
    return proc.stdout


# ── 驱动脚本：每个状态一段，点完之后等渲染稳定 ──────────────────────────
HELPERS = r"""
const sleep = (ms) => new Promise(r => setTimeout(r, ms));
const frame = () => document.querySelector('[data-slot="root"] > *');
const collapsed = () => frame().hasAttribute('data-sidebar-collapsed');
const sideBtns = () => [...document.querySelectorAll('[data-slot="sidebar"] button, [data-slot="sidebar"] [role="button"]')];
const byLabel = (re) => sideBtns().find(b => re.test(b.getAttribute('aria-label') || ''));
async function openSidebar() {
  if (!collapsed()) return 'already';
  const b = byLabel(/打开侧边栏|open sidebar/i) || sideBtns()[0];
  if (b) b.click();
  for (let i = 0; i < 20 && collapsed(); i++) await sleep(150);
  return collapsed() ? 'FAILED' : 'ok';
}
async function closeSidebar() {
  if (collapsed()) return 'already';
  const b = byLabel(/收起侧边栏|collapse sidebar/i) || sideBtns()[0];
  if (b) b.click();
  for (let i = 0; i < 20 && !collapsed(); i++) await sleep(150);
  return collapsed() ? 'ok' : 'FAILED';
}
const treeItems = () => [...document.querySelectorAll('[role="treeitem"]')];
const sessionRow = () => treeItems().find(r => (r.getAttribute('data-row-key')||'').startsWith('session:'));
const wsRow = () => treeItems().find(r => (r.getAttribute('data-row-key')||'').startsWith('workspace:'));
"""

STATES = [
    ("home", f"{HELPERS}\n(async()=>{{ await closeSidebar(); await sleep(600); return 'ok' }})()"),
    ("workspaces", f"{HELPERS}\n(async()=>{{ await openSidebar(); await sleep(700); return 'ok' }})()"),
    ("ws-expanded", f"""{HELPERS}
(async()=>{{
  await openSidebar();
  const w = wsRow();
  if (w) {{ w.click(); await sleep(700); }}
  return JSON.stringify({{ws: w ? (w.innerText||'').trim().slice(0,20) : null}});
}})()"""),
    ("chat", f"""{HELPERS}
(async()=>{{
  await openSidebar();
  let s = sessionRow();
  if (!s) {{ const all = treeItems().filter(r=>(r.getAttribute('data-row-key')||'').startsWith('workspace:'));
            for (const w of all) {{ w.click(); await sleep(300); s = sessionRow(); if (s) break; }} }}
  if (!s) return 'no session row';
  s.click();
  await sleep(2600);
  return 'ok';
}})()"""),
    ("trajectory", f"""{HELPERS}
(async()=>{{
  const t = [...document.querySelectorAll('[role="tab"]')].find(x => /轨迹|trajectory/i.test(x.innerText||''));
  if (!t) return 'no tab';
  t.click(); await sleep(1200); return 'ok';
}})()"""),
    ("tool-open", f"""{HELPERS}
(async()=>{{
  const t = [...document.querySelectorAll('[role="tab"]')].find(x => /对话|chat/i.test(x.innerText||''));
  if (t) {{ t.click(); await sleep(900); }}
  const rows = [...document.querySelectorAll('[data-disclosure-row]')].filter(r => r.getAttribute('data-expandable') !== 'false');
  for (const r of rows.slice(0, 3)) {{ r.click(); await sleep(500); }}
  return JSON.stringify({{expanded: rows.length}});
}})()"""),
    ("search", f"""{HELPERS}
(async()=>{{
  await openSidebar(); await sleep(500);
  const b = sideBtns().find(x => /搜索会话|search/i.test(x.getAttribute('aria-label')||''));
  if (!b) return 'no search btn';
  b.click(); await sleep(1000); return 'ok';
}})()"""),
    ("rightbar", f"""{HELPERS}
(async()=>{{
  await closeSidebar(); await sleep(400);
  const b = [...document.querySelectorAll('button')].find(x => /右侧|right sidebar|rightbar/i.test(x.getAttribute('aria-label')||''));
  if (!b) return 'no rightbar btn';
  b.click(); await sleep(1200); return 'ok';
}})()"""),
    ("final", f"{HELPERS}\n(async()=>{{ await closeSidebar(); await sleep(400); return 'ok' }})()"),
]


def audit() -> dict:
    out = subprocess.run(
        [sys.executable, str(ROOT / "scripts" / "audit_page.py")]
        + (["--serial", SERIAL] if SERIAL else []) + ["--json"],
        capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=300)
    t = out.stdout
    i = t.find("{")
    return json.loads(t[i:]) if i != -1 else {}


def main() -> int:
    SHOTS.mkdir(parents=True, exist_ok=True)
    rows = []
    for name, js in STATES:
        # ⚠️ 驱动**只能跑一次**：上面曾经写成 `cdp(...) if cdp else ""` 再跟一句
        #    无条件调用，等于把同一段脚本执行两遍 —— 点两次「☰」等于
        #    打开再关掉，页面停在错误状态，报告全是假的。
        drive = cdp(None, js).strip().splitlines()[-1]
        shot = SHOTS / f"{name}.png"
        cdp(None, path=str(shot))
        d = audit()
        i = d.get("info", {})
        kinds: dict[str, int] = {}
        for p in d.get("problems", []):
            kinds[p["kind"]] = kinds.get(p["kind"], 0) + 1
        rows.append({
            "state": name, "drive": drive[:40],
            "vw": d.get("viewport", {}).get("w"),
            "nodes": i.get("visibleNodes"),
            "header": i.get("headerH"),
            "high": d.get("sev", {}).get("high", 0),
            "mid": d.get("sev", {}).get("mid", 0),
            "ovf": d.get("overflowX"),
            "kinds": kinds,
            "top": [f"{p['kind']}:{p['msg'][:34]}" for p in d.get("problems", [])[:3]],
        })
        print(f"✓ {name:<12} high={rows[-1]['high']:<3} mid={rows[-1]['mid']:<4} "
              f"顶栏={i.get('headerH')} 节点={i.get('visibleNodes')}")

    print("\n=== 汇总 ===")
    tot_h = sum(r["high"] for r in rows)
    tot_m = sum(r["mid"] for r in rows)
    print(f"共 {len(rows)} 个状态，high {tot_h} / mid {tot_m}")
    for r in rows:
        if r["top"]:
            print(f"  {r['state']:<12} {r['kinds']}")
            for t in r["top"]:
                print(f"      {t}")
    if "--shots" in sys.argv:
        print("\n截图:", ", ".join(str(p.name) for p in sorted(SHOTS.glob("*.png"))))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
