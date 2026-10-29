# 09 · 侧边菜单全模块功能分析

> 分析日期：2026-10-05 ｜ 范围：`src/components/SideNav.svelte` 定义的 10 个侧边模块 + 外壳架构
> 编排：主会话（外壳/情报/插件/设置） + 子 Agent（文件/游戏/环境+联网+DSH/笔记+远程）

## 模块清单与报告索引

| #   | 侧边项 | nav id     | 主入口                                      | 报告                                         | 负责     |
| --- | ------ | ---------- | ------------------------------------------- | -------------------------------------------- | -------- |
| 1   | 环境   | `env`      | `src/features/env/EnvPage.svelte`           | [01-环境-env.md](./01-环境-env.md)           | 子 Agent |
| 2   | DSH    | `dsh`      | `src/features/dsh/DshPage.svelte`           | [04-DSH-dsh.md](./04-DSH-dsh.md)             | 子 Agent |
| 3   | 文件   | `files`    | `src/features/search/SearchPage.svelte`     | [03-文件-files.md](./03-文件-files.md)       | 子 Agent |
| 4   | 远程   | `remote`   | `src/features/remote/RemotePage.svelte`     | [06-远程-remote.md](./06-远程-remote.md)     | 子 Agent |
| 5   | 笔记   | `notes`    | `src/features/notes/NotesPage.svelte`       | [05-笔记-notes.md](./05-笔记-notes.md)       | 子 Agent |
| 6   | 联网   | `web`      | `src/features/web/WebPage.svelte`           | [02-联网-web.md](./02-联网-web.md)           | 子 Agent |
| 7   | 情报   | `intel`    | `src/features/intel/IntelPage.svelte`       | [07-情报-intel.md](./07-情报-intel.md)       | 主会话   |
| 8   | 游戏   | `games`    | `src/features/games/GamesPage.svelte`       | [08-游戏-games.md](./08-游戏-games.md)       | 子 Agent |
| 9   | 插件   | `plugins`  | `src/features/plugins/PluginsPage.svelte`   | [09-插件-plugins.md](./09-插件-plugins.md)   | 主会话   |
| 10  | 设置   | `settings` | `src/features/settings/SettingsPage.svelte` | [10-设置-settings.md](./10-设置-settings.md) | 主会话   |

汇总报告：见本目录上级或本目录的汇总文档。

## 侧栏机制说明（分析基线）

- 侧栏为 **48px 纯图标窄栏**（`src/components/SideNav.svelte:92`），无文字标签，位置感由左缘指示条 `qx-nav-active` + 图标同色承担。
- 9 个主导航项 + `settings` 沉底（`SideNav.svelte:121`，`mt-auto` + 分隔线，与工具项分组）。
- **keep-alive 容器**：已访问页面只做 `visibility: hidden` 显隐，不销毁（`src/App.svelte` `show()`；未访问页面不挂载，DSH 额外后台预热）。
- **可分离视图**：仅 `dsh` 支持右键「分离到独立窗口」（`SideNav.svelte:65` `DETACHABLE_VIEWS`），spawn 成功才让位，窗口关闭经 `window://closed` 事件恢复。
- 导航状态极简：`nav.page`（当前页）+ `nav.visited`（挂载过）+ `nav.detached`（已分离），见 `src/stores/nav.svelte.ts`。
- **无键盘导航**：侧栏为纯 `<button>` 列表，未实现方向键/数字键快捷跳转。
