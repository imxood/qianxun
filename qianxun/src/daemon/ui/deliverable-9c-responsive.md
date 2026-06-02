# Stage 9c — Web 响应式 + 错误边界 + 离线检测 (Task C)

千寻 Web Console Stage 9c C.1–C.5 全部落地. 详细报告见
`~/.mavis/plans/plan_fd1762bc/outputs/webui-stage9c-responsive/deliverable.md`.

## 改动一览

### 新增文件

- `src/lib/stores/ui.svelte.ts` — sidebar drawer 状态 + afterNavigate 自动关
- `src/lib/stores/connection.svelte.ts` — daemon health check (3s AbortController)
- `src/routes/+error.svelte` — SvelteKit 错误页
- `src/lib/stages-9c.test.ts` — 12 个 vitest 单元测试
- `scripts/csp-e2e.mjs` — CSP 端到端验证脚本
- `tests/e2e/responsive.spec.ts` — 4 个 Playwright 响应式测试 (待 Playwright 装好可跑)
- `src/test-stubs/app-navigation.ts` — vitest $app/navigation stub

### 修改文件

- `src/lib/components/layout/Sidebar.svelte` — drawer + backdrop + connection dot
- `src/lib/components/layout/TopBar.svelte` — 汉堡按钮 + 复用 connectionStore
- `src/routes/+layout.svelte` — svelte:boundary + failed snippet + connection banner
- `vitest.config.ts` — 加 $app/navigation alias
- `src/lib/components/chat/MessageBubble.svelte` — 修 type guard (sibling chat task 遗留)
- `src/routes/settings/+page.svelte` — 移除不存在 Github icon (sibling settings task 遗留)
- `Cargo.toml` (workspace) — `tower-http` features 加 `set-header`
- `qianxun/src/daemon/router.rs` — CSP layer + `stage9c_csp_tests` 模块

## 验证

```
$ pnpm run check       # 0 errors 0 warnings
$ pnpm vitest run      # 12 files / 136 tests pass
$ cargo test stage9c_csp_tests   # 2 passed; 0 failed
$ node scripts/csp-e2e.mjs        # CSP present on / and /v1/system/health
```

## 验证清单 (task spec)

- [x] Sidebar 加 `lg:` 断点 class
- [x] TopBar 加汉堡按钮 (移动端显示)
- [x] +error.svelte 存在
- [x] lib/stores/connection.svelte.ts 存在 + health check 逻辑
- [x] 主页 +layout.svelte 加 connection banner
- [x] daemon 端 CSP header 配置 + 1 个 e2e 测试
- [x] vitest ≥ 10 新测试 (Stage 9c 12)
- [x] pnpm check 0/0
- [x] git commit 1
- [ ] Lighthouse mobile 模拟 — 待 Playwright 装好环境后跑 (spec 已写)
