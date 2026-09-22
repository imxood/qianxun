# qx-websearch — 千寻内置联网搜索

千寻安装包自带的联网能力包（DSH 插件 + CLI 引擎链）：

- agent 会话的原生 `web_search` 跑在本包引擎链上：Firecrawl 免注册免费通道开箱即用，可配置 Antigravity CLI / Tavily / Exa，多密钥轮换 + 配额冷却自动故障转移
- `read_page`：带焦点读单页，返回结构化证据（summary / 正文提取 / 外链 / uncertainty / warnings），默认拦截私网目标
- `x_search`：X（推特）检索；Grok CLI 不可用时显式标注降级为网页顶替
- 设置卡片挂在 DSH 网页「设置 → 插件」，写入 `~/.qianxun/websearch/config.json`（与 CLI `config set` 共用一份）

## 布局

- `src/` CLI 引擎链源码（vite 打包为自包含的 `dist/main.js`，运行时零外部依赖）
- `dsh/` DSH 插件：宿主半段 `index.js`（provider 注册 + x_search/read_page 工具 + 设置路由）与浏览器半段 `client.js`（设置卡片），均为零依赖纯文件
- `skills/` 多宿主 skill 文档；`output-schema.test.ts` 保证文档与 schema 的 lockstep

## 构建与同步

```sh
pnpm build:websearch   # 独立 workspace 内 install + build（不并外壳工具链）
pnpm sync:websearch    # build 并同步进 src-tauri/src/websearch/assets（随安装包内嵌）
pnpm --dir packages/websearch test   # 本包测试（vitest，456 用例）
```

## 部署模型（宿主托管）

与 qx-bridge 同款：包**不进** pnpm 依赖、**不进** `dsh.profile.bundles`。千寻启动时把 `src-tauri/src/websearch/assets` 幂等落盘到 `<DSH_HOME>/profiles/web/node_modules/qx-websearch/`，并在 profile `cordis.patch.yml` 写两行：

- `- id: web / config: searchProvider: qx-websearch`（web 接缝改道本包）
- `- insert: [id: qx-websearch, name: qx-websearch]`（挂载；`exports["."]` 装载宿主半段，`dsh.client` 声明装载设置卡）

注意：任何对 profile 的 `pnpm install/remove` 都会剪掉这种手工落盘的目录——千寻每次启动的 `websearch::ensure` 会自动补齐，无需手工干预。

## 安全

- 本地抓取器在任何请求前拒绝：私网/保留 IPv4/IPv6（含 `::ffff:` 映射）、云元数据端点、内嵌凭据 URL、非 http(s) 协议；每跳重定向重查
- DNS 重绑定钉死：校验解析出的每个地址并返回批准 IP，连接经自定义 lookup 钉在该 IP（Host/SNI 保留主机名）
- `198.18.0.0/15` 的 **DNS 解析结果**视为代理 fake-ip 占位值放行（TUN/Clash 环境开箱即用）；URL 直写该段仍拦
- 密钥 0600 落盘、不进 argv；`config show` 全视图脱敏；错误信息统一脱敏后才进日志与工具输出；浏览器只见 hasKey 永不见密钥
