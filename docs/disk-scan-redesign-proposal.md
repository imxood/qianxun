# Disk Scan — Interaction Redesign Proposal

> Scope: `src/features/search/disk/DiskScan.svelte` (entry → scan → live → done → errors).
> Backend `src-tauri/src/disk.rs` already does what we need (rotating cancel flag,
> throttled ~100 ms `Progress` frames with partial `topChildren`, terminal `Done`).
> No backend changes are proposed; this is a frontend interaction pass.

---

## TL;DR (the two changes the user actually asked for)

1. **Stop auto-scanning on first visit.** Replace the `else { scan(home.root, 'reset') }`
   branch at `DiskScan.svelte:359–362` with an explicit empty state + primary CTA.
   Keep the SWR branch at `DiskScan.svelte:353–358` (returning user, cached tree,
   `background:true`) — that path is already correct.
2. **Make treemap blocks visibly grow during scan.** The data plumbing is fine
   (`liveGrow` at `DiskScan.svelte:196–204` already rewrites sizes every 200 ms),
   but the rendered blocks have no CSS transition on geometry, so they snap. Add
   `transition-[left,top,width,height,background] duration-200 ease-out` on the
   block `<button>` (around `DiskScan.svelte:956–986`) and a one-time fade-in for
   children that did not exist on the previous frame.

Everything below operationalises those two changes plus the smaller polish items
the user listed.

---

## 1. Entry state (first visit, no auto-scan)

**Trigger:** `onMount` reaches the `else` branch at `DiskScan.svelte:359–362`
and today immediately calls `scan(home.root, 'reset')`. The replacement is
"render an empty state and wait for a click".

### What the user sees

Replace the current "准备扫描…" line at `DiskScan.svelte:941–942` with a
centered empty-state card filling the `<div class="qx-card …">` at
`DiskScan.svelte:940`:

- **Icon (top, subtle):** a single monochrome folder/disk SVG at 96–112 px, in
  `var(--qx-accent)` at ~30 % opacity. No emoji. The icon sits in a 120 × 120
  rounded-square background tile (`bg-accent-soft`) for visual anchor.
- **Title:** "扫描数据目录以分析占用" (h2, `text-fg`).
- **Subtitle:** `千寻的数据目录位于 {home.root}。点击下方按钮开始扫描，
或选择其它目录。` — `text-sm text-muted`, monospace path.
- **Primary CTA:** `qx-btn qx-btn-primary qx-btn-md`, label
  **"扫描数据目录"** — full-width within the card, ~280 px wide. Clicking
  calls `scan(home.root, 'reset')`. This is the one the user asked for.
- **Secondary action:** `qx-btn qx-btn-ghost`, label **"选择其它目录…"** —
  calls `pickExternal()` (already wired at `DiskScan.svelte:376–382`).
- **Below the card (outside it, in the empty space):** if `externals.length > 0`,
  a horizontal row of compact chips under the heading "或扫描最近用过的目录"。
  Each chip reuses the existing pill style from `DiskScan.svelte:872–895` but
  slightly smaller (h-6). If `externals.length === 0`, this row is hidden.

### Why an illustration AND a CTA

- DaisyDisk and WinDirStat both use a hero illustration on empty. Without it
  the page reads as "broken". An illustration also signals _intentional_ empty,
  not a load failure.
- The CTA must be primary (filled, accent-coloured) — DaisyDisk's "Scan a
  folder" and TreeSize's "Scan" both put it center, full-width.

### Returning user (cached `session.trail`)

Keep current SWR. Cached tree shows immediately; `background:true` scan
refreshes silently. The "刚扫描过 / N 分钟前扫描" badge from
`agoText` (`DiskScan.svelte:138–145`) tells the user the data isn't fresh.

### External-directories bar on the empty state

Show it (the chips). It is the fastest path back into a non-default directory
and DaisyDisk/TreeSize both keep "recent folders" visible at empty. After the
first scan, it lives at its current position above the treemap.

---

## 2. Scan trigger (idle / scanning / error)

Three states for the **same button slot** (currently two buttons at
`DiskScan.svelte:849–869`):

### Idle, has data

- Top-right toolbar as today: **视图切换 | 重新扫描 | 添加目录**. The
  refresh button is `qx-btn-outline` (secondary) — scanning again is the
  exception, not the rule.
- The toolbar is hidden entirely on the empty state. The primary CTA lives
  inside the empty-state card instead.

### Idle, no data (empty state)

- Toolbar hidden. The card holds the primary CTA only.
- **添加目录** is reachable via the secondary "选择其它目录…" ghost button
  in the card (same `pickExternal` handler).

### Scanning (foreground)

- Toolbar stays visible. **停止** button appears at the right end
  (`DiskScan.svelte:860–869`); label toggles to `停止中…` while the cancel
  RPC is in flight (`stopping` flag, `DiskScan.svelte:867`).
- **重新扫描** is _not_ disabled — clicking it rotates to a fresh scan on the
  same root (this is what `scan(current.entry.path, 'refresh')` at
  `DiskScan.svelte:853` already does, via `seq++` + `DiskScanManager.rotate`
  in `disk.rs:162–172`).
- **添加目录** also rotates — useful for "I started the wrong folder" flow.

### Scanning (background, SWR refresh on return)

- No top-right **停止** — `scanning && background` already hides the toolbar
  stop button because `{#if scanning}` is true but… wait, the current code
  _does_ show it (`DiskScan.svelte:860`). Fix: gate the stop button on
  `scanning && !background`. Background refreshes are uninterruptible from
  the user's perspective (they may not even know one is happening).
- The progress line at `DiskScan.svelte:902–916` already disambiguates via
  `(后台刷新 · {scanningRoot})` (line 913). Good.

### Error

- A horizontal error strip across the page (`DiskScan.svelte:897–899`
  currently) gains an inline **重试** ghost button on the right that calls
  `scan(scanningRoot || home.root, 'reset')`. Today the strip is text-only,
  so the user has to scroll up and re-click the refresh button.
- The empty-state card itself gets a red left-border variant for the case
  where the empty state _is_ the error (e.g., `pickExternal` chose a path
  that no longer exists — `disk.rs:200` returns `目录不存在`). The card then
  reads:
  - Title becomes `无法扫描此目录` in `text-danger`.
  - Subtitle: the backend's error message verbatim.
  - Single CTA: **重试** (calls `scan(home.root, 'reset')`) plus the
    secondary **选择其它目录…** stays.

### Cancellation

Already correct. `stopScan` at `DiskScan.svelte:279–287` sets the backend's
atomic flag; backend emits a final `Done{cancelled:true}` frame
(`disk.rs:233–261`); frontend records `partial: {files, dirs}` at
`DiskScan.svelte:253`. The "部分结果" badge and footer note at
`DiskScan.svelte:820–827` and `DiskScan.svelte:1122–1135` already communicate
this.

### Mid-scan cancellation visual

Today the progress line just disappears when the scan ends. Add a 600 ms
fade-out on the progress row, so the "停止" press feels acknowledged.

---

## 3. Live feedback during scan ("blocks visibly grow")

This is the bug at `DiskScan.svelte:196–204` (the `liveGrow` function) +
`DiskScan.svelte:595–610` (the `blockStyle` output) +
`DiskScan.svelte:955–986` (the block `<button>` markup).

### Root cause (concrete)

- `liveGrow` does mutate `top.entry.size` and `top.entry.children` correctly.
  The `$derived.by` chain at `DiskScan.svelte:495–524` (`sortedChildren`,
  `visibleChildren`) and `DiskScan.svelte:580–593` (`blocks`) therefore
  recompute. `squarify` (`treemap.ts:31–88`) re-runs and writes new
  `Rect` objects.
- The CSS class on the block `<button>` is
  `class="group absolute overflow-hidden rounded-[3px] text-left transition-[filter] …"`
  (line 957). `transition-[filter]` only animates `filter`. `left`, `top`,
  `width`, `height` _snap_.
- Result: every 200 ms the layout reflows with no in-between frame. To the
  eye the blocks look like they shuffle, not grow.

### Fix — visual rules

Apply these to the block `<button>` (replacing line 957's class):

1. **Geometry transition** — add `transition-[left,top,width,height,background,opacity]`
   `duration-200` `ease-out`. Tailwind will compile this to the four
   geometry properties plus `background-color` and `opacity`, all over
   200 ms with `ease-out`. (`will-change: left, top, width, height` should
   be set on the parent `.absolute.inset-0` to avoid layout thrash on the
   remaining ~250 blocks; the GPU only needs to repaint, not relayout.)
2. **Size update policy** — keep the existing 200 ms throttle at
   `DiskScan.svelte:200` (`if (stamp - lastLive < 200) return`). Throttle
   duration should equal transition duration (200 ms / 200 ms) so an
   in-flight transition never gets interrupted mid-frame. If you change
   one, change the other.
3. **"Grow ≥ X %" rule** — current code rewrites the entire children array
   every tick. To make the growth feel deliberate, only re-render a child
   when its `size` changed by ≥ 2 % _or_ ≥ 64 KB _or_ ≥ 250 ms has passed
   since its last update. (Same throttle already does this globally; the
   per-child guard is for the rare case where one big block grows fast
   while others are stable — keeps the squarify from reshuffling.)
4. **New children fade-in** — children whose `path` did not exist in the
   previous `visibleChildren` should mount at `opacity: 0` and animate to
   `opacity: 1` over 200 ms. Implementation: keep a `prevPaths: Set<string>`
   ref alongside `liveGrow`; on each tick, items not in the set get
   `style="opacity:0; animation: qx-grow-in 200ms ease-out forwards"` with
   a `@keyframes` rule defined globally. After the tick, the new set
   replaces the previous. (Svelte's `{#each ... (key)}` at line 955 will
   mount a fresh node for each genuinely new path, which lets us hook the
   CSS animation without breaking the keyed reconciliation.)
5. **Label threshold** — `showLabel` at `DiskScan.svelte:612–614` flips a
   label on/off when `w ≥ 72 && h ≥ 34`. Wrap the label `<span>` with
   `transition-opacity duration-150` so it fades in over 150 ms instead of
   popping. Same for the size text.
6. **The first frame guard** — until the first `Progress` frame arrives
   _or_ `Done` arrives, do **not** render any blocks at the new layer
   (`DiskScan.svelte:580–582`). Render a thin skeleton (a 1 px outline at
   `bg-bg` with `animate-pulse`) covering the treemap area. This kills
   the "flash from empty to giant" on fast scans.

### Throttle policy in one sentence

> Block for a directory resizes smoothly when its partial size grows ≥ 5 %
> of its current value **or** ≥ 200 ms has passed since its last resize
> (whichever is later); new children fade in over 200 ms; the entire
> treemap fades from skeleton to real blocks on first frame.

### What stays the same

- Backend cadence (~100 ms frames).
- Throttle at `DiskScan.svelte:200`.
- Display cap (`DISPLAY_LIMIT = 300`, `DiskScan.svelte:96`). 200–300 blocks
  is well under the per-frame compositor budget for smooth 200 ms
  transitions on modern hardware.
- Squarify algorithm. It _will_ re-shuffle when ratios change a lot — the
  CSS transition turns that "shuffle" into a "morph", which reads as
  growth. Do not chase layout-stable squarify variants.

---

## 4. Completion

### Stable final layout

- When the `Done` frame arrives, `trail` is replaced with the full tree
  (`DiskScan.svelte:257`). The CSS transitions defined above interpolate
  each block from its last live position to its final position.
- If the final sizes differ meaningfully from the last progress frame
  (they always will, by a few percent), the user sees a 200 ms ease-out
  settle. No flash, no jump.
- A 200 ms cross-fade is _not_ needed in the common case. If you ever
  swap from "live" to a freshly-restored cached tree that has very
  different ratios, that's the one case where a `view-transition`-style
  fade helps; out of scope here.

### Summary numbers

Already in place at `DiskScan.svelte:815–828`:

- `formatBytes(current.entry.size) · {n} 项 · {agoText(current.at)}`
- `部分结果` badge when `current.partial` is set.

Two micro-improvements:

1. When `current.skipped > 0`, surface a small badge on the right of the
   summary row (currently only the footer at `DiskScan.svelte:1129–1133`
   shows it). One tap to expand an explanation.
2. When `Date.now() - current.at > 6 hours`, prefix the summary with a
   subtle `⚠ 数据可能已过期` (SVG icon, not emoji) and make it a single
   click to refresh.

### When "Refresh" becomes available

Always. The **重新扫描** button is never disabled in the toolbar
(`DiskScan.svelte:849–856`). On the empty state, the CTA card is the
refresh. The clean separation is: **toolbar = has-data actions,
empty-state card = first-scan CTA**.

### Background refresh state → foreground

When `session.trail` is non-empty on entry and a background refresh is in
flight, the treemap is fully visible (no `dimClass` because `dimClass` at
`DiskScan.svelte:121` evaluates to empty string when `background` is
true). Good — that's correct. Just don't show the **停止** button for
background scans (see §2).

---

## 5. Edge interactions

### Switching target mid-scan

Current flow: clicking an external chip → `scan(path, 'reset')` at
`DiskScan.svelte:381`, which `++seq`s, sets `scanning = true`, replaces
`trail = [provisionalItem(path)]`, and starts a new channel. Old frames
are dropped by `if (ticket !== seq) return` at `DiskScan.svelte:232`.
Backend rotates via `DiskScanManager.rotate` (`disk.rs:162–172`).

Improvements:

- Add a 1.5 s toast `已切换到 {pathTail(external)}` (`showToast` already
  wired at `DiskScan.svelte:755–762`) so the user understands what
  happened to the scan that was running.
- During the brief gap between switching and the first new progress frame
  (~100–200 ms), the treemap is in the empty state for the new path.
  Show a thin "准备切换…" overlay instead of an empty box.

### Returning to root

`goHome()` at `DiskScan.svelte:345–347` calls `scan(home.root, 'reset')`
unconditionally. That re-scans the entire data directory even when the
user just clicked the wrong breadcrumb.

Better:

```
function goHome() {
  if (trail.length === 1 && normPath(current?.entry.path) === normPath(home.root)) return;
  // Walk back through the trail first; only rescan if there's nothing cached.
  if (trail.length === 0) { scan(home.root, 'reset'); return; }
  crumbTo(0);
  // If the root item is stale (>6 h), kick a background refresh.
  const root = trail[0];
  if (root && Date.now() - root.at > 6 * 3600_000) scan(home.root, 'refresh', { background: true });
}
```

This is the single highest-leverage micro-fix in the file.

### Partial results from a cancelled scan

Already preserved and labelled:

- `current.partial` carries `{files, dirs}` (`DiskScan.svelte:253`).
- "部分结果" badge in the summary row (`DiskScan.svelte:820–827`).
- Footer explanation (`DiskScan.svelte:1122–1135`).

Add one more affordance: in the footer, append a **"继续扫描"** ghost
button next to the "重新扫描" toolbar button — labelled differently so
the user knows it's the _same_ scan resuming, not a fresh one. Internally
it can call `scan(current.entry.path, 'refresh')`; the user-facing
distinction is just the label.

### Resize during scan

`ResizeObserver` at `DiskScan.svelte:564–572` already updates `boxSize`,
which causes `blocks` to recompute. The CSS transition makes the resize
smooth. Good. Verify no layout thrash — `will-change: left, top, width,
height` on the treemap container.

---

## 6. Empty / error states

### Directory not found

- Backend `disk_scan_stream` returns `Err` (`disk.rs:199–201`) which the
  front-end `.catch` at `DiskScan.svelte:266–275` turns into `actionError`.
- The empty-state card now has the error variant described in §2, so the
  user sees `无法扫描此目录` + the path + a **重试** button.
- If the path was a recently added external, also surface a "从最近目录中移除"
  action so the chip doesn't keep offering a dead path.

### Permission errors

- Per-entry permission failures are counted into `current.skipped` and
  shown in the footer (`DiskScan.svelte:1129–1133`). Good.
- Whole-directory unreadable (`std::fs::read_dir` returns Err →
  `walk_size` returns 0 → entry has `size: 0, children: []`):
  - Today this renders as "空目录" at `DiskScan.svelte:943–944`. That's
    misleading.
  - Detect it: if `entry.dir && entry.children.length === 0 && !scanning`
    _and_ the entry's metadata came back as unreadable, render
    "无权限访问此目录" with a one-line hint about Windows ACLs / macOS
    TCC. (Cheap detection: backend could surface an `error` flag per
    `DiskEntry`; out of scope for this proposal but a one-line addition
    to `disk.rs:43–54` would do it.)

### Very fast scans (< 200 ms)

The risk: blocks appear empty, then jump to full size with no in-between.
The first-frame guard in §3 (skeleton until first frame) handles this.
Specifically:

- Render the skeleton overlay only while `scanning && !progress`.
- The instant `progress` arrives _or_ `Done` arrives, the skeleton is
  removed and the real blocks mount at their target sizes. CSS animation
  `qx-grow-in` plays once on mount (200 ms ease-out).
- If the scan completes before the skeleton is even shown (sub-50 ms),
  the user sees the skeleton flash for one paint and then real blocks
  appear. Acceptable — far better than the empty-then-giant snap.

---

## 7. Industry comparison

| Tool              | Empty state                                                 | Live growth                                                                                | Cancel                                | Target switch                               | Pitfall                                                       |
| ----------------- | ----------------------------------------------------------- | ------------------------------------------------------------------------------------------ | ------------------------------------- | ------------------------------------------- | ------------------------------------------------------------- |
| **WinDirStat**    | None — opens a "select folder" dialog.                      | No. Scans complete, then renders.                                                          | Cancellable.                          | Always rescan from dialog.                  | No live feedback; slow drives feel hung.                      |
| **DaisyDisk**     | Beautiful animated disk illustration + "Scan a folder" CTA. | No. Ring sweeps the _whole_ disk surface during scan; treemap appears when scan completes. | Cancellable, partial result retained. | Click a different disk segment on the ring. | Treemap "pops" in at the end (no live morph).                 |
| **TreeSize**      | "Select folder" dialog + recent paths.                      | **Yes** — explicit "live treemap" mode where blocks grow as scan progresses.               | Cancellable, partial retained.        | Dropdown.                                   | Live mode can flash if scan completes before the first paint. |
| **千寻 today**    | "准备扫描…" text, auto-scan starts.                         | Partial (data updates, no visual transition).                                              | Already implemented.                  | Already implemented.                        | Blocks shuffle, not grow; auto-scan surprises the user.       |
| **千寻 proposed** | Illustration + primary CTA + recent dirs.                   | **Yes** — CSS transitions on geometry + first-frame skeleton.                              | Already implemented + 600 ms fade.    | Same + confirmation toast.                  | (Avoid by) skeleton-on-empty.                                 |

### Takeaways we apply

1. **Don't auto-scan.** DaisyDisk and WinDirStat both gate entry on a click.
   TreeSize is the only one that starts automatically and it has been
   criticised for it in user reviews for over a decade.
2. **Live growth is good — but only with smooth transitions.** TreeSize's
   live mode is what users want; its flash bug is what we must avoid via
   the first-frame skeleton.
3. **Cancel + partial retention is table stakes.** We already do this.

---

## 8. Direct answers to the four questions

1. **Empty illustration?** Yes — a monochrome folder/disk SVG (no emoji) at
   96–112 px in `var(--qx-accent)` @ ~30 % opacity, inside a soft tile.
2. **Primary "扫描数据目录" CTA?** Yes — `qx-btn qx-btn-primary qx-btn-md`,
   centered in the empty-state card. Click calls `scan(home.root, 'reset')`.
   Secondary ghost action: "选择其它目录…" → `pickExternal()`.
3. **External-directories bar on empty state?** Yes, but in a compact form
   directly under the empty-state card: a row of small chips labelled
   "或扫描最近用过的目录". Once any scan completes, the bar moves to its
   current position above the treemap. If `externals.length === 0` it is
   hidden.
4. **Scan / refresh button placement?**
   - Idle, no data → empty-state card (primary CTA).
   - Idle, has data → top-right toolbar (current placement, lines 849–859).
   - Scanning, foreground → toolbar + **停止** button at far right (lines
     860–869). Add the small **刷新中…** badge in the summary row when the
     scan is `background:true`.
   - Error → inline error strip with **重试**, plus an error variant of the
     empty-state card if there's nothing else to render.

---

## 9. File change list (anchored to current line numbers)

| Where                                                  | What                                                                                                                                                                                                              |
| ------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `DiskScan.svelte:349–368` (`onMount`)                  | Delete the auto-scan `else` branch. Always wait for a user click on first visit. Keep the SWR branch.                                                                                                             |
| `DiskScan.svelte:345–347` (`goHome`)                   | Prefer in-memory trail walk-back over `scan(home.root, 'reset')`. Background refresh only when stale.                                                                                                             |
| `DiskScan.svelte:941–944` (empty / 空目录 placeholder) | Replace with the empty-state card described in §1.                                                                                                                                                                |
| `DiskScan.svelte:849–869` (toolbar)                    | Hide toolbar when `!current`; gate the **停止** button on `!background`.                                                                                                                                          |
| `DiskScan.svelte:860`                                  | Change `{#if scanning}` → `{#if scanning && !background}` for the stop button.                                                                                                                                    |
| `DiskScan.svelte:897–899` (`actionError` strip)        | Add inline **重试** ghost button.                                                                                                                                                                                 |
| `DiskScan.svelte:196–204` (`liveGrow`)                 | No logic change; document the 200 ms = transition-duration pairing.                                                                                                                                               |
| `DiskScan.svelte:580–582` (`blocks` derived)           | Add `&& progress === null` guard: while `scanning && !progress`, return `[]` and let a sibling skeleton render.                                                                                                   |
| `DiskScan.svelte:956–986` (block markup)               | Replace `transition-[filter]` with `transition-[left,top,width,height,background,opacity] duration-200 ease-out` + `will-change` on the container + fade-in animation for newly-keyed children + fade for labels. |
| New `@keyframes qx-grow-in`                            | Global CSS; opacity 0 → 1, scale 0.96 → 1, 200 ms ease-out.                                                                                                                                                       |
| `DiskScan.svelte:1122–1135` (footer)                   | Append **继续扫描** ghost button next to **重新扫描** when `current.partial` is set.                                                                                                                              |

No backend changes required. `disk.rs` already does the right thing.

---

## 10. Open questions (worth confirming before coding)

1. **Reduced motion.** Add `@media (prefers-reduced-motion: reduce)` to
   collapse the 200 ms transitions to 0 ms? (Recommended — `qx-grow-in`
   should respect it.)
2. **First-frame skeleton copy.** Skeleton-only, or "正在枚举目录…" text?
3. **Mid-scan switch toast.** Confirm copy: `已切换到 {pathTail(external)}`?
4. **goHome behaviour.** Confirm the "walk back through trail if possible"
   semantics — this is a behaviour change that affects power users.
