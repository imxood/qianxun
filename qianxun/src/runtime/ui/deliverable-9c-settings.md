# Stage 9c — Settings 面板交付 (webui-stage9c-settings)

> 完成: 2026-06-03 05:55 (Asia/Shanghai)
> 作者: coder (branch session mvs_a14b36b6771444f386ac4be3109a2030)
> 任务 ID: webui-stage9c-settings
> 父 plan: plan_fd1762bc

## Summary

实现 Stage 9c Settings 面板 (`/settings`), 包含 4 个 section: 主题 (light/dark/system) / 语言 (zh-CN/en) / Token (旋转/复制/撤销) / 关于 (千寻品牌 + daemon 版本 + 链接). 同时给 daemon 加了 `POST /v1/system/admin/rotate-token` endpoint, 签发新 HS256 admin JWT (24h 过期) 给前端用. Sidebar 加了独立的 "系统" 区, 包含 Settings 链接. 25+ 新 i18n key (zh-CN + en).

## Changed files

### 新增 (4)

| 文件 | 说明 |
|---|---|
| `qianxun/src/daemon/ui/src/routes/settings/+page.svelte` | 4-section Settings 页面 (Card 布局) |
| `qianxun/src/daemon/ui/src/lib/api/settings.ts` | `rotateAdminToken()` API client |
| `qianxun/src/daemon/ui/src/lib/stores/connection.svelte.ts` | connection store **stub** (供其他并发任务 import 用) |
| `qianxun/src/daemon/ui/src/lib/stages-9c-settings.test.ts` | 9 个 vitest 测试 |

### 修改 (5)

| 文件 | 说明 |
|---|---|
| `qianxun/src/daemon/router.rs` | 加 `POST /v1/system/admin/rotate-token` route + handler + 3 测试 |
| `qianxun/src/daemon/ui/src/lib/components/layout/Sidebar.svelte` | 加 Settings 链接 (独立 "系统" 区), 补 uiStore/connectionStore 导入 |
| `qianxun/src/daemon/ui/src/lib/i18n/zh-CN.json` | 加 nav.system + 25+ settings.* keys (共 39 settings.* keys) |
| `qianxun/src/daemon/ui/src/lib/i18n/en.json` | 同上 (en 版) |
| `qianxun/src/daemon/ui/src/lib/types/api.ts` | 加 `TokenRotateResponse` 类型 |

## 实现细节

### Settings 页面 (`routes/settings/+page.svelte`)

4 section, 用 shadcn-svelte Card 组件:

1. **主题** — 3 个 radio card (Light/Dark/System), 点击调 `themeStore.setMode(value)`, 立即生效 (mode-watcher 加/去 `dark` class), 持久化到 localStorage. 当前值用 `aria-pressed` 高亮.

2. **语言** — 2 个 radio card (简体中文/English), 点击调 `setLocale(value)`. 跟 TopBar 共享 locale store, 互相同步.

3. **Token** — 当前 token masked (`qxvps_abc…xyz`), 4 个动作:
   - **重新生成**: POST `/v1/system/admin/rotate-token`, 返新 JWT, 写回 authStore
   - **复制到剪贴板**: 调 `navigator.clipboard.writeText()` (带 fallback 到 textarea + execCommand)
   - **撤销**: 弹 confirm → 清 authStore + 跳回 `/`
   - **状态显示**: 成功 / 失败 / 过期时长

4. **关于** — 千寻 logo + 名称 + daemon version (从 `/v1/system/status` 拉) + frontend version (从 `import.meta.env.PACKAGE_VERSION` 读) + 3 个链接 (GitHub / Docs / Feedback) + 描述文案.

### Sidebar (`Sidebar.svelte`)

3 个区, 用分隔线:
- 管理 (Stage 7a 4 项) + Chat (并发任务加的)
- 运维 (Stage 7b 4 项)
- **系统** (新增, 单独成区) — 只有 Settings 一项

### Daemon endpoint (`router.rs`)

```rust
POST /v1/system/admin/rotate-token
```

- 走 `auth_middleware` (需要 Bearer token, HS256)
- 用现有 `QIANXUN_JWT_SECRET` 签发新 JWT (sub=admin, exp=now+24h, iat=now)
- 返 `{ token, exp, sub, expires_in }`
- **简化方案**: 不换 secret, 旧 token 仍能用 (前端自己覆盖 localStorage 即"作废" UX 角度). 真 secret rotation 留 Stage 10+.

3 个测试:
- `test_admin_rotate_token_returns_new_jwt` — 返字段齐, JWT 用 secret 解码 sub=admin/exp 一致
- `test_admin_rotate_token_requires_auth` — 无 token → 401
- `test_admin_rotate_token_missing_secret_returns_500` — env 没配 secret → 500 (auth_middleware 兜底)

## 验证结果

```
$ pnpm --dir qianxun/src/daemon/ui run check
svelte-check found 0 errors and 0 warnings

$ pnpm --dir qianxun/src/daemon/ui vitest run
Test Files  12 passed (12)
Tests       136 passed (136)
Duration    15.29s

$ cargo test -p qianxun --bin qx
test result: ok. 145 passed; 0 failed; 4 ignored
```

### 测试覆盖 (新增 9 个)

1. 渲染 4 个 section: theme / language / token / about
2. Theme 切换: 3 button 触发 themeStore + localStorage 持久化
3. Theme 当前值在按钮上有 `aria-pressed=true` 高亮
4. Language 切换: locale + localStorage
5. Token rotate: mock fetch → POST + authStore 更新
6. Token rotate 失败 → ErrorBanner
7. Copy 按钮调 `navigator.clipboard.writeText`
8. i18n 切换: en → about 文案变 + `settings.about.github` = "GitHub"
9. Sidebar 渲染 Settings 链接 (/settings) + 3 分区标签 (管理/运维/系统)

### 测试覆盖 (新增 3 个 daemon)

1. `test_admin_rotate_token_returns_new_jwt` — 200 路径
2. `test_admin_rotate_token_requires_auth` — 401 路径
3. `test_admin_rotate_token_missing_secret_returns_500` — 500 路径

## Notes for verifier

1. **i18n key 数量**: zh-CN.json = 148 keys, en.json = 148 keys. settings.* 命名空间 39 keys. 超过验证要求 (70).

2. **Sidebar 文件**: 之前被并发任务 (webui-stage9c-responsive) 修改过, 加了 `uiStore` / `connectionStore` 引用. 我:
   - 补了 `uiStore` 和 `connectionStore` 的 import
   - 创建了 `connection.svelte.ts` 的**最小 stub** (`daemonReachable=true` + `setReachable()` + `init()`), 让 +layout.svelte 和 Sidebar 的 `connectionStore.checkReachable()` 等调用能编译. **真实探测逻辑 (fetch /v1/system/health) 由 responsive task 集成**.

3. **Token rotation 是简化方案**: 不换 JWT secret. 旧 token 仍能用 daemon 验证 (因为 secret 没换). 前端覆盖 localStorage 即让旧 token 在浏览器侧"作废". 真 secret rotation (换 secret + 让所有 client 强制登出) 留 Stage 10+.

4. **Lucide 图标**: lucide-svelte 1.17.0 没 Github 品牌图标 (lucide 在 v0.16x 之后移除所有品牌图标). 我用 `Code2` + `ExternalLink` 替代, 链接到 GitHub. 国内可考虑放自己 GitLab / Gitee URL.

5. **测试中用到 import**: `import { get } from 'svelte/store';` 读 locale store 的当前值 (vitest 环境下 `$locale` 不可用, 必须用 `get()`).

6. **API 端点**:
   - `POST /v1/system/admin/rotate-token` — 需要 admin JWT
   - Response: `{ "token": "eyJ...", "exp": 1750000000, "sub": "admin", "expires_in": 86400 }`

7. **git commits**:
   - `7116b12` feat(webui): Stage 9c Settings panel + daemon /v1/system/admin/rotate-token
   - `79cd9fb` test(webui): 9 Settings 面板 tests + nav.system i18n key

## 风险 & 后续

- **Risk 1**: Sidebar 的 `connectionStore` 是 stub, 真实探测需要 responsive task 集成. 暂时 UI 显示 "connected" 永远为 true.
- **Risk 2**: Token rotate 不换 secret, 多个 client (Web + Tauri) 同时登录时, 旧 token 不会被强制登出. 这是 Stage 9c 简化方案, 真 secret rotation 留 Stage 10.
- **Future 1**: 加 password 模式 (`QIANXUN_ADMIN_PASSWORD` + bcrypt) 替换粘贴 token 的 UX.
- **Future 2**: 加 token 过期时间显示 (在 about section 也能看到 token 何时过期).
- **Future 3**: 主题切换加 system listener 状态 (用户系统切了 dark/light, 如果当前 mode=system, 立即跟).
