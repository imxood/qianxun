# qianxun 项目 — 知识库 (Mavis 维护)

> Mavis (orchestrator) 跟 sibling agent 共享的项目级知识.
> 编辑后 commit 一下, 让团队都能用.

## 1. Daemon admin 密码 (每次 e2e 验证前查)

- **密码文件**: `E:\git\maxu\qianxun\.opencode\password.txt` (git ignored)
- **当前密码**: `VVgAOQsdw8uHNT1t9fC29Q`
- **更新机制**: 用户重启 daemon 时 daemon 会重生成 admin.cred, 密码会变. 密码文件**可能**跟最新同步, 也可能没同步 (用户说"我重新启动了 daemon" 后没主动更新文件).
- **使用流程**:
  1. e2e 验证 (e.g. playwright 16 page click 验证) 前, 读 `password.txt` 拿当前密码
  2. `curl -X POST /v1/auth/login -d '{"password":"..."}'` 拿 token
  3. JWT 解析 exp + sub, 写 3 localStorage keys: `qianxun_admin_token` + `qianxun_admin_token_exp` + `qianxun_admin_sub`
  4. 跑验证
- **坑**: 用户给过错密码 (16:30 用户说 "w75SUidOXhhyR1u1csdGfw" 但实际是更早的, daemon 重启后 16:30 这密码也失效). 16:59 用户说"看 password.txt 文件". 后续每次重启 daemon 都先读 password.txt, 不要再问用户.

## 2. Daemon 启动方式 (默认是 dev mode)

- 用户默认用 `qx --daemon --port 23900 --ui-dev http://127.0.0.1:5174` (main.rs `ui_dev` 字段 `default_value = "http://127.0.0.1:5174"`)
- dev mode 走 vite 反代 (5174), SvelteKit 2.61 客户端 router base 公式固定返 `new URL(".", location).pathname.slice(0, -1)`, 跟 `paths.base` 脱钩. dev mode 浏览器访问 `/ui` 404
- **prod mode** (Mavis e2e 验证用): `qx --daemon --port 23900 --ui-dev "" --ui-dist <build_path>`
  - `--ui-dev ""` 显式覆盖默认 dev mode
  - `--ui-dist <build_path>` 走静态 build
  - 配合 daemon router `redirect_ui_no_slash` middleware (`/ui` → `/ui/`)

## 3. mavis mcp 验证基础设施 (prod mode)

- 必备 3 件套:
  1. **prod build** (`cd qianxun/src/daemon/ui && pnpm build`)
  2. **playwright MCP** (自加 `pw_test` server, 不要用内置 `playwright` 标 "Not connected"):
     ```
     mavis mcp add pw_test '{"command":"cmd","args":["/c","npx","-y","@playwright/mcp@0.0.70"]}'
     ```
  3. **3 localStorage keys** (token + exp + sub), 缺一不可
- 14 page verify 14/14 pass 模板: `scratch/verify_16_pages_v2.py`
- 13 page click verify 模板: `scratch/click_verify_13pages.py` (每 page 1 核心 click + console check)
- click 触发 5s timeout 修法: 用 `threading.Thread + join(2.5s)` fire-and-forget

## 4. 关键 bug 修复 (跨 session 记得)

- `qianxun/src/daemon/router.rs` 加 `redirect_ui_no_slash` middleware (`/ui` → `/ui/` 308)
- `qianxun/src/daemon/ui/svelte.config.js` `paths.base='/ui' + paths.relative=true` (build 注入)
- `qianxun/src/daemon/ui/src/routes/+layout.svelte` `authStore.init()` 提到 `<script>` 顶层 (修 Svelte 5 child-first mount 401)
- `qianxun/src/daemon/ui/src/routes/system/+page.svelte` `metrics.cpu_percent?.toFixed(1) ?? '—'` (修 Windows undefined toFixed)

## 5. docs 经验总结

- `docs/30_子项目规划/05-stage-12-verification-retro.md` — Stage 12 路径统一 + 16 page 验证, 8 章节经验
