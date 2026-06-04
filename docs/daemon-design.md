# 千寻 Daemon 模式设计

> 版本: 0.2 | 更新: 2026-06-01 | 状态: 已实现（骨架）
>
> Daemon HTTP 框架已实现：axum 路由 + 会话管理 + SSE 端点。API Key 管理、BudgetManager、WS Client 待补全。

---

### 1.1 CLI 参数

```
qx daemon [OPTIONS]

选项：
  --port <PORT>          HTTP 监听端口（默认 23900，覆盖 config.json 的 daemon.port）
  --config <PATH>        配置文件路径（默认 ~/.qianxun/config.json）
  -v, --verbose          详细日志输出
  --daemon-url <URL>     连接远程 Daemon（用于 CLI 作为客户端时，默认 http://127.0.0.1:23900）
```

### 1.2 文件结构（当前实现）

```
qianxun/src/daemon/             # Daemon 子命令（在单二进制内）
├── mod.rs                      # 模块入口 + pub async fn run() + AppState
├── router.rs                   # axum 路由定义 + 全部 handler
│   ├── GET  /v1/system/health  # 健康检查
│   ├── GET  /v1/system/status  # 状态概览
│   ├── POST /v1/chat/session   # 创建会话
│   ├── GET/DELETE /v1/chat/session/:id  # 获取/删除会话
│   ├── POST /v1/chat/session/:id/prompt  # SSE 流式 Prompt
│   ├── GET  /v1/tools          # 工具列表
│   ├── GET  /v1/config         # 配置读取
│   ├── GET/POST /v1/memory/*    # 记忆管理（stub）
│   ├── GET  /v1/skills         # 技能列表
│   └── GET/POST /v1/mcp/servers # MCP 管理（stub）
└── agent_host.rs               # AgentLoopHost（会话管理）
    ├── create_session()        # 创建会话（sess_YYYYMMDD_HHMMSS_uuuuuu）
    ├── session_exists()
    └── delete_session()

# 待实现（设计文档已有规划）:
# - keychain.rs: API Key 加密存储
# - budget.rs: Token 预算管理
# - ws_client.rs: VPS WS Client
# - service.rs: systemd/Windows Service 注册
```


## 1. 设计目标

### 核心理念

> **Daemon 是千寻唯一的运行时依赖 —— CLI、ACP、Web UI 都是它的前端。**

| 目标 | 说明 |
|---|---|
| **统一 AgentLoop** | 所有 AgentLoop 状态在 Daemon 进程中，CLI/ACP 不持有对话 |
| **集中密钥管理** | 全部 API Key 在 Daemon 加密持有，前端不接触 |
| **全局预算** | Token 用量跨会话统一追踪和限流 |
| **会话持久化** | AgentLoop 的对话状态可以持久化到 SQLite |
| **多前端透明** | CLI、ACP、Web UI 共享同一个 AgentLoop 实例 |
| **健康自愈** | systemd / Windows Service 管理 + 优雅关闭 |

### 非目标

- 多机器分布式 AgentLoop（未来 Phase）
- Daemon 集群（单机单实例）

---

## 2. 架构

### 2.1 进程结构

```
┌─────────────────────────────────────────────────────────┐
│  qx daemon                                         │
│                                                         │
│  ┌─────────────────────────────────────────────────┐    │
│  │  HTTP Server (axum, 127.0.0.1:23900)            │    │
│  │  ├─ /v1/chat/*        AgentLoop 代理            │    │
│  │  ├─ /v1/llm/*         LLM Provider 管理         │    │
│  │  ├─ /v1/memory/*      记忆管理                   │    │
│  │  ├─ /v1/tools/*       工具执行（MCP 转发）      │    │
│  │  ├─ /v1/skills/*      技能管理                   │    │
│  │  ├─ /v1/config/*      配置管理                   │    │
│  │  ├─ /v1/session/*     会话管理                   │    │
│  │  ├─ /v1/system/*      系统状态                   │    │
│  │  └─ /ui/*             Web UI 静态文件            │    │
│  └─────────────────────────────────────────────────┘    │
│                                                         │
│  ┌─────────────────────────────────────────────────┐    │
│  │  AgentLoop 实例池                               │    │
│  │  ├─ Conversation (active)  ← 当前活跃对话       │    │
│  │  ├─ Conversation (paused)  ← 已暂停的对话        │    │
│  │  └─ max_sessions: 10       ← 并发上限            │    │
│  └─────────────────────────────────────────────────┘    │
│                                                         │
│  ┌─────────────────────────────────────────────────┐    │
│  │  LLM Provider Pool                              │    │
│  │  ├─ DeepSeekProvider (default, encrypted key)   │    │
│  │  ├─ OpenAIProvider    (optional, encrypted key) │    │
│  │  └─ health_check → 每 60s 检测可用性            │    │
│  └─────────────────────────────────────────────────┘    │
│                                                         │
│  ┌─────────────────────────────────────────────────┐    │
│  │  MemoryCore（直接链接 qianxun-memory）          │    │
│  └─────────────────────────────────────────────────┘    │
│                                                         │
│  ┌─────────────────────────────────────────────────┐    │
│  │  ToolRegistry                                   │    │
│  │  ├─ builtin: 5 tools (直接引用)                 │    │
│  │  ├─ MCP: N tools (通过 McpServerManager)        │    │
│  │  └─ Skill: Agent 通过 skill_read 间接调用       │    │
│  └─────────────────────────────────────────────────┘    │
│                                                         │
│  ┌─────────────────────────────────────────────────┐    │
│  │  VPS WS Client (可选)                           │    │
│  │  └─ 连接 VPS 节点发现/命令中转                   │    │
│  └─────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────┘
```

### 2.2 与当前 CLI/ACP 的关系

```
当前（Phase 1-2）：                  目标（Phase 4）：
┌──────────┐  ┌──────────┐         ┌──────────┐  ┌──────────┐
│ CLI      │  │ ACP      │         │ CLI      │  │ ACP      │
│ AgentLoop│  │ AgentLoop│         │ 薄终端    │  │ 协议桥   │
│ API Key  │  │ API Key  │         └────┬─────┘  └────┬─────┘
│ Memory   │  │ Memory   │              │HTTP :23900    │HTTP :23900
└──────────┘  └──────────┘              └──────┬────────┘
                                               │
                                        ┌──────┴──────┐
                                        │   Daemon     │
                                        │  AgentLoop   │
                                        │  API Key     │
                                        │  MemoryCore  │
                                        └─────────────┘
```

### 2.3 迁移策略

从当前 CLI 独立模式到 Daemon 的过渡路径：

```
Phase 2 → Phase 3（过渡，可选 Daemon）
  CLI: 内嵌 AgentLoop（如当前）
  └─ 同时支持 --daemon-url 参数连接外部 Daemon

Phase 3（推荐 Daemon）
  CLI: 默认连接本地 Daemon，若无 Daemon 则报错提示启动
  └─ 仍保留 --standalone 降级模式（但标记为 deprecated）

Phase 4（强制 Daemon）
  CLI: 纯薄客户端，不再支持 --standalone
  Daemon: 作为 systemd / Windows Service 自动启动
```

### 2.4 VPS WS Client 心跳

Daemon 与 VPS 的 WebSocket 连接使用 **30 秒 ping/pong** 维持活性：

```
Daemon → VPS: {"type":"ping"}
VPS → Daemon: {"type":"pong"}

超时：60 秒未收到 pong → 判定断线
  → 尝试重连，指数退避：1s / 2s / 4s / 8s（上限 60s）
  → 重连成功后重新注册能力列表
  → 离线期间本地 AgentLoop 照常工作
```

WS Client 在 Daemon 启动流程的步骤 5 中初始化（见 §10）。

### 2.5 全局配置

Daemon 的全局配置用 `~/.qianxun/config.json`：

```json
{
  daemon: {
    host: "127.0.0.1",
    port: 23900,
    max_sessions: 10,
    session_timeout_min: 60,       // 会话无活动超时
  },

  // LLM providers（API Key 不保存在此文件，通过密钥链获取）
  providers: [{
    name: "deepseek",
    api_base: "https://api.deepseek.com/anthropic/v1",
    default_model: "deepseek-v4-flash",
    models: [
      { id: "deepseek-v4-flash", max_tokens: 128000 },
      { id: "deepseek-v4", max_tokens: 128000 },
    ],
    caps: ["chat", "streaming", "thinking"],
  }],

  agent: {
    max_turns: 50,
    max_retries: 3,
    max_tokens: 4096,
    temperature: 0.0,
  },

  budget: {
    max_input_tokens: 128000,
    max_output_tokens: 128000,
    max_daily_cost: 1.0,          // 每日最大花费（USD）
  },

  memory: { /* see memory-design.md */ },
  mcp_servers: [ /* see mcp-design.md */ ],
  skills: { /* see skills-design.md */ },
}
```

---

## 3. HTTP 框架

### 3.1 框架选型

| 框架 | 评价 |
|---|---|
| **axum**（选定） | tokio 原生、 tower 中间件生态、千寻已有 reqwest（tokio 栈） |
| actix-web | ❌ 引入 actix 运行时，与 tokio 冲突 |
| salvo | ⚠️ 生态较小，社区支持不足 |

**选 axum** 的原因：
- 与 tokio 原生集成（千寻整个异步栈都基于 tokio）
- tower Service 抽象便于加中间件（日志、限流、认证）
- 社区主流，文档齐全

### 3.2 中间件栈

```
Request
  │
  ├─ TowerLayer::Timeout(30s)         ← 请求级别超时
  ├─ TowerLayer::Trace               ← tracing 请求追踪
  ├─ TowerLayer::Cors                ← 本地开发 CORS（/ui 用）
  │
  ├─ 路由匹配
  │   ├─ /v1/* → ApiRouter
  │   │   ├─ RateLimit（按 IP 限流）
  │   │   └─ Auth（除 /v1/system/health 外全部需要）
  │   │
  │   └─ /ui/* → StaticFileRouter（Web UI）
  │
  └─ Response
```

### 3.3 认证

Daemon 监听 `127.0.0.1:23900`（仅本地访问），不需要 TLS 或复杂认证。但为了防止其他本地进程滥用，增加简单 Token 认证：

```json
{
  daemon: {
    // 如果设置，CLI/ACP 需要在 HTTP Header 中携带此 Token
    access_token: "auto-generated-uuid",  // 首次启动自动生成
  }
}
```

CLI 读取 `~/.qianxun/daemon.token` 自动附加到请求头。

### 3.4 路由定义

```rust
// === daemon/router.rs

pub fn build_router(state: AppState) -> Router {
    Router::new()
        // LLM
        .route("/v1/llm/chat", post(chat_handler))
        .route("/v1/llm/embed", post(embed_handler))
        .route("/v1/llm/providers", get(list_providers).post(add_provider))
        .route("/v1/llm/providers/:name", put(update_provider).delete(delete_provider))
        .route("/v1/llm/providers/:name/test", post(test_provider))
        
        // Chat / AgentLoop
        .route("/v1/chat/session", post(create_session))
        .route("/v1/chat/session/:id", get(get_session).delete(delete_session))
        .route("/v1/chat/session/:id/prompt", post(prompt_handler)) // SSE 流式
        
        // Memory
        .route("/v1/memory/*", memory_routes())
        
        // Tools
        .route("/v1/tools", get(list_tools))
        .route("/v1/tools/call", post(call_tool))
        
        // MCP
        .route("/v1/mcp/servers", get(list_mcp_servers).post(add_mcp_server))
        
        // Skills
        .route("/v1/skills", get(list_skills))
        
        // Config
        .route("/v1/config", get(get_config).put(update_config))
        
        // System
        .route("/v1/system/health", get(health_handler))
        .route("/v1/system/status", get(status_handler))
        .route("/v1/system/restart", post(restart_handler))
        .route("/v1/system/shutdown", post(shutdown_handler))
        
        // Web UI
        .nest_service("/ui", ServeDir::new("web/dist"))
        
        .layer(TimeoutLayer::new(Duration::from_secs(30)))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
```

### 3.5 SSE 流式响应格式

`POST /v1/chat/session/:id/prompt` 的响应通过 **Server-Sent Events** 流式返回。CLI/ACP 按以下格式解析：

```
event: text
data: {"text": "你好，我是千寻"}

event: tool_call
data: {"id": "toolu_abc", "name": "read_file", "arguments": {"path": "src/main.rs"}}

event: tool_result
data: {"id": "toolu_abc", "name": "read_file", "content": "fn main() {\n..."}

event: error
data: {"code": "rate_limit", "message": "请求过快"}

event: turn_finished
data: {"reason": "end_turn", "usage": {"input": 123, "output": 456}}
```

| 事件 | 触发时机 | data 字段 |
|---|---|---|
| `text` | LLM 输出文本块 | `{text: string}` |
| `thinking` | LLM 思考块（DeepSeek 特有） | `{text: string}` |
| `tool_call` | LLM 请求调用工具 | `{id, name, arguments}` |
| `tool_result` | 工具执行完成 | `{id, name, content}` |
| `error` | 发生错误 | `{code, message}` |
| `turn_finished` | 一轮 LLM 调用结束 | `{reason, usage}` |

CLI 侧解析：每行 `event: xxx` 后跟一行 `data: xxx`，空行分隔事件。使用 `eventsource-stream` crate 或手动解析。

---

## 4. AgentLoop 实例管理

### 4.1 会话生命周期

```
POST /v1/chat/session               → 创建会话 → 返回 session_id
POST /v1/chat/session/:id/prompt    → 发送 prompt，SSE 流式返回
GET  /v1/chat/session/:id           → 获取会话状态和对话历史
DELETE /v1/chat/session/:id         → 关闭/销毁会话
```

### 4.2 AgentLoopHost

```rust
// === daemon/agent_host.rs

pub struct AgentLoopHost {
    sessions: Arc<RwLock<HashMap<SessionId, SessionHandle>>>,
    max_sessions: usize,
}

struct SessionHandle {
    id: SessionId,
    conversation: Conversation,
    created_at: Instant,
    last_active: Instant,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl AgentLoopHost {
    /// 创建新会话
    /// 
    /// 多个 qx 实例可同时连接同一个 Daemon：
    ///   - 每个实例创建独立的 Session（对话隔离）
    ///   - 所有实例共享同一个 MemoryCore（记忆互通）
    ///   - 一个实例 crash 不影响其他实例
    pub async fn create_session(&self, config: SessionConfig) -> Result<SessionId> {
        let sessions = self.sessions.read().unwrap();
        if sessions.len() >= self.max_sessions {
            return Err(DaemonError::MaxSessionsReached);
        }
        drop(sessions);
        
        // SessionId 格式：sess_YYYYMMDD_HHMMSS_uuuuuu
        // 按时间排序可追溯，微秒精度确保单 Daemon 内唯一
        let id = generate_session_id();
        let conversation = Conversation::new(config.system_prompt);
        
        self.sessions.write().unwrap().insert(id.clone(), SessionHandle {
            id: id.clone(),
            conversation,
            created_at: Instant::now(),
            last_active: Instant::now(),
            shutdown_tx: None,
        });
        
        Ok(id)
    }

/// 生成可读的 Session ID
/// 
/// 格式: sess_YYYYMMDD_HHMMSS_uuuuuu
/// 示例: sess_20260531_142530_123456
fn generate_session_id() -> String {
    let now = chrono::Utc::now();
    format!("sess_{}", now.format("%Y%m%d_%H%M%S_%6f"))
}
    
    /// 处理 prompt（SSE 流式）
    pub async fn handle_prompt(
        &self,
        session_id: &str,
        prompt: String,
        sink: impl OutputSink,
    ) -> Result<()> {
        let handle = self.get_session(session_id)?;
        handle.last_active = Instant::now();
        
        // Conversation 包含整个消息历史，深拷贝开销随对话轮次增长。
        // 未来优化方向：用 Arc<Vec<Message>> + Copy-on-Write 避免大对话时的频繁克隆。
        let mut conversation = handle.conversation.clone();
        // ... 与当前 processing_loop 相同，只是数据来源从 stdin 变为 HTTP body
        processing_loop::handle_user_message(&mut conversation, &prompt, sink).await
    }
    
    /// 清理过期会话（后台任务）
    pub async fn reap_stale_sessions(&self) {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let mut sessions = self.sessions.write().unwrap();
            let timeout = Duration::from_secs(60 * 60); // 1 小时
            sessions.retain(|_, h| {
                let keep = h.last_active.elapsed() < timeout;
                if !keep {
                    info!("reaping stale session {}", h.id);
                }
                keep
            });
        }
    }
}
```

### 4.3 与 processing_loop 的关系

`processing_loop::handle_user_message()` 作为 `qianxun-core` 库函数，**不关心调用者是谁**。在 Phase 3 开发期由 CLI standalone 进程直接调用，在 Phase 4 由 Daemon HTTP handler 调用。核心代码不变。

| 组件 | Phase 3（CLI standalone） | Phase 4（Daemon） |
|---|---|---|
| `handle_user_message()` | `qianxun-core/agent/engine.rs` | 同一函数，位置不变 |
| `Conversation` | `qx` 进程内存 | Daemon 进程内存（+ 可选快照持久化） |
| `OutputSink` | `CliOutputSink` | `AcpOutputSink` / `SseOutputSink` |
| 调用者 | `cli/run.rs` | HTTP handler（`/v1/chat/session/:id/prompt`） |

核心 `agent/engine.rs` 的代码不需要重复编写——它始终在 `qianxun-core` 中，CLI standalone 和 Daemon 共用。

---

## 5. API Key 管理

### 5.1 密钥链集成

```rust
pub trait KeychainProvider: Send + Sync {
    fn name(&self) -> &str;
    fn set(&self, service: &str, key: &str) -> Result<()>;
    fn get(&self, service: &str) -> Result<Option<String>>;
    fn delete(&self, service: &str) -> Result<()>;
}
```

平台实现：

| 平台 | 实现 | crate |
|---|---|---|
| macOS | macOS Keychain | `security-framework` |
| Linux | secret-tool (libsecret) | `secret-service` 或调用 `secret-tool` CLI |
| Windows | Credential Manager | `winapi` 或 `keyring` crate |

**建议**：使用 `keyring` crate（v3.x），它统一封装了三个平台的密钥链 API。

```rust
// 使用 keyring crate
use keyring::Entry;

// 存储
let entry = Entry::new("qianxun", "deepseek_api_key")?;
entry.set_password("sk-xxxx")?;

// 读取
let key = entry.get_password()?;
```

### 5.2 启动时的 API Key 解析

```
Daemon 启动
  │
  ├─ 读取 providers 配置（不含 api_key）
  │
  ├─ for each provider:
  │     ├─ 尝试密钥链读取：keyring::Entry::new("qianxun", "{name}_api_key")
  │     ├─ 如果密钥链不存在：
  │     │   ├─ 检查环境变量 DEEPSEEK_API_KEY
  │     │   └─ 如果环境变量也不存在 → 启动时不加载此 provider
  │     └─ Provider 可以配置，但运行时没有 key → Agent 调用返回 LlmError::NoApiKey
  │
  └─ 所有配置的 provider 就绪（或标记为 key missing）
```

### 5.3 API Key 设置流程

```
CLI/ACP 用户设置 API Key：
  qx daemon config set-provider-key deepseek sk-xxxx
    │
    ├─ POST /v1/llm/providers/deepseek/key
    │   Body: { "api_key": "sk-xxxx" }
    │
    ├─ Daemon 收到后：
    │   ├─ 验证：尝试一次 API 调用（/chat 或 /models）
    │   ├─ 通过验证 → keyring::Entry::set_password()
    │   └─ 验证失败 → 返回错误，不保存
    │
    └─ 成功 → 此 provider 立即可用
```

---

## 6. Token 预算和限流

### 6.1 全局 BudgetManager

```rust
// === daemon/budget.rs

pub struct BudgetManager {
    daily_spent: Arc<AtomicU64>,       // 今日已用 token
    daily_reset: Instant,              // 下次重置时间
    max_daily_cost: f64,               // 最大日花费（USD）
    
    concurrent_requests: Arc<AtomicU32>,  // 当前并发数
    max_concurrent: u32,                  // 最大并发（默认 5）
}

impl BudgetManager {
    /// 检查是否可以发起请求
    pub fn try_acquire(&self, estimated_tokens: u32) -> Result<(), BudgetError> {
        if self.concurrent_requests.load(Ordering::Relaxed) >= self.max_concurrent {
            return Err(BudgetError::TooManyConcurrent);
        }
        
        // 检查日预算（简化：token → USD 粗糙转换）
        let cost = estimated_tokens as f64 * 0.000002; // $2/1M tokens
        if self.daily_spent.load(Ordering::Relaxed) as f64 + cost > self.max_daily_cost {
            return Err(BudgetError::DailyBudgetExceeded);
        }
        
        self.concurrent_requests.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    
    /// 请求完成后释放
    pub fn release(&self, tokens_used: u32) {
        self.concurrent_requests.fetch_sub(1, Ordering::Relaxed);
        self.daily_spent.fetch_add(tokens_used as u64, Ordering::Relaxed);
    }
}
```

### 6.2 限流策略

| 维度 | 限制 | 行为 |
|---|---|---|
| 并发请求数 | 最大 5 个 | 超出返回 429 Too Many Requests |
| 日预算 | `budget.max_daily_cost` | 超出返回 429，提示"日预算已用尽" |
| 单请求 max_tokens | `agent.max_tokens` | LLM 侧限制 |
| Provider 速率限制 | 由各 Provider 错误处理 | 自动退避重试 |

---

## 7. 优雅关闭

```
Daemon 收到 SIGTERM / Ctrl+C
  │
  ├─ 1. HTTP Server 停止接受新连接（graceful_shutdown）
  │
  ├─ 2. 通知所有活跃会话：
  │     ├─ SSE 连接发送 shutdown 事件
  │     ├─ 等待正在执行的 tool 完成（最多 5s）
  │     └─ 未完成的 LLM 请求丢弃
  │
  ├─ 3. 持久化活跃会话快照：
  │     ├─ Conversation → SQLite（可选，依赖 Phase 3 Memory）
  │     └─ 标记 session 状态为 "paused"
  │
  ├─ 4. 关闭 MCP 子进程：
  │     ├─ 发送 SIGTERM
  │     └─ 等待 3s → SIGKILL
  │
  ├─ 5. 关闭 MemoryCore：
  │     └─ SQLite checkpoint + 关闭
  │
  └─ 6. 退出（exit 0）
```

```rust
// 使用 tokio 的 graceful shutdown
let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(());

// 监听系统信号
tokio::spawn(async move {
    tokio::signal::ctrl_c().await.unwrap();
    info!("收到 SIGINT，开始优雅关闭...");
    let _ = shutdown_tx.send(());
});

// axum 的 graceful shutdown
let listener = tokio::net::TcpListener::bind("127.0.0.1:23900").await.unwrap();
axum::serve(listener, app)
    .with_graceful_shutdown(async move {
        shutdown_rx.changed().await.ok();
    })
    .await
    .unwrap();
```

---

## 8. 系统服务注册

### 8.1 Linux (systemd --user)

```
~/.config/systemd/user/qx-daemon.service:
  [Unit]
  Description=千寻 Daemon - Personal AI Assistant
  After=network-online.target
  
  [Service]
  Type=simple
  ExecStart=%h/.cargo/bin/qx daemon
  Restart=on-failure
  RestartSec=5
  Environment=RUST_LOG=info
  
  [Install]
  WantedBy=default.target
```

安装命令：`qx daemon install` → 写入此文件 → `systemctl --user daemon-reload && systemctl --user enable --now qx-daemon`

### 8.2 Windows (Windows Service)

使用 `windows-service` crate：

```rust
use windows_service::{
    service::{ServiceAccess, ServiceErrorControl, ServiceStartType, ServiceType},
    service_manager::{ServiceManager, ServiceManagerAccess},
};

fn install_windows_service() -> Result<()> {
    let manager = ServiceManager::new(
        None,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE
    )?;
    
    let service = manager.create_service(
        "qianxun-daemon",
        ServiceAccess::CHANGE_CONFIG | ServiceAccess::START,
        ServiceType::OWN_PROCESS,
        ServiceStartType::AutoStart,
        ServiceErrorControl::Normal,
        std::env::current_exe()?.to_str().unwrap(),
        None,
        None,
        "千寻 Daemon - Personal AI Assistant",
    )?;
    
    service.start()?;
    Ok(())
}
```

---

## 9. 与 ACP 协议的关系

### 9.1 当前 ACP 模式

```
当前：qx --acp-mode
  ACP Server 在 CLI 进程中（与 AgentLoop 同进程）
  stdio JSON-RPC 2.0 ↔ Zed 编辑器

迁移后：
  │ Zed 编辑器 │          │ qx acp (薄代理) │         │ Daemon │
  │             │ stdio   │                 │ HTTP    │        │
  │ ACP Client  │←──────→│  ACP→HTTP 桥接   │←──────→│ Agent  │
  │ (plugin)    │         │                 │ :23900  │ Loop   │
  └─────────────┘         └─────────────────┘         └────────┘
```

`qx acp` 变成一个无状态代理：接收 Zed 的 JSON-RPC 请求，转换为 HTTP 请求转发到 Daemon，再将 Daemon 的 SSE 流式响应转换为 ACP session/update 通知。

### 9.2 ACP 转发逻辑

```rust
// ACP 协议桥的精简版：不再持有 AgentLoop，只做协议转发
// 只做协议转换：

async fn handle_prompt(req: PromptRequest) -> Result<()> {
    // 1. 创建 Daemon 会话
    let session = http_client
        .post("http://127.0.0.1:23900/v1/chat/session")
        .json(&CreateSessionRequest { project, workspace })
        .send().await?;
    
    // 2. 转发 prompt（SSE 流）
    let mut stream = http_client
        .post(format!("http://127.0.0.1:23900/v1/chat/session/{}/prompt", session.id))
        .json(&PromptBody { messages, tools })
        .send().await?
        .bytes_stream();
    
    // 3. SSE → ACP notification 转换
    while let Some(chunk) = stream.next().await {
        let event = parse_sse_event(chunk?);
        send_acp_notification(event).await?;
    }
}
```

---

## 10. 启动流程（完整）

```
qx daemon
  │
  ├─ 1. 解析 CLI 参数 → config 路径
  │
  ├─ 2. 读取 ~/.qianxun/config.json
  │
  ├─ 3. 初始化日志（tracing）
  │
  ├─ 4. 初始化密钥链连接
  │     └─ 加载所有 Provider 的 API Key
  │
  ├─ 5. 初始化 MemoryCore
  │     ├─ 打开 SQLite
  │     ├─ 重建向量索引
  │     └─ FTS5 就绪
  │
  ├─ 6. 初始化 AgentLoopHost
  │     └─ 创建空会话池
  │
  ├─ 7. 初始化 ToolRegistry
  │     ├─ 注册 5 个 builtin 工具
  │     ├─ 初始化 McpServerManager
  │     │   └─ 启动所有 auto_start=true 的 MCP 服务
  │     └─ 注册 MCP 工具的 McpToolWrapper
  │
  ├─ 8. 初始化 SkillManager
  │     └─ load_all() + DAG 验证
  │
  ├─ 9. 初始化 BudgetManager
  │
  ├─ 10. 启动 HTTP Server (axum)
  │
  ├─ 11. 可选：连接 VPS
  │     ├─ 读取 ~/.qianxun/daemon.token
  │     └─ WS connect
  │
  ├─ 12. 后台任务
  │     ├─ 会话过期清理（60s 间隔）
  │     ├─ MCP 空闲关闭（30 分钟）
  │     ├─ 成员健康检查（60s 间隔）
  │     └─ 文件监视（skills 目录）
  │
  └─ 13. 等待信号（SIGTERM/Ctrl+C）
        └─ 优雅关闭
```

---

## 11. 依赖清单

```toml
# qianxun/Cargo.toml（dependencies 段）
[dependencies]
qianxun-core = { path = "../qianxun-core" }
qianxun-memory = { path = "../qianxun-memory" }

axum = "0.8"                          # HTTP 框架
tower = "0.5"                         # 中间件
tower-http = { version = "0.6", features = ["cors", "trace", "timeout", "fs"] }
tokio = { workspace = true, features = ["full"] }
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
tracing = { workspace = true }
anyhow = { workspace = true }
chrono = { workspace = true }

# 密钥链
keyring = "3"

# 系统服务注册
# Windows: windows-service = "0.7"
# Linux: 直接写 systemd unit 文件（不需要 crate）

# 信号处理（tokio 自带）
```

---

## 12. 测试策略

| 测试类型 | 覆盖 |
|---|---|
| 单元测试 | BudgetManager 预算计算和并发控制 |
| 单元测试 | AgentLoopHost 会话创建/过期清理 |
| 集成测试 | HTTP API 端点响应和 SSE 流式输出 |
| 集成测试 | 密钥链读写（mock keyring） |
| 容错测试 | Provider 不可用时的降级行为 |
| 容错测试 | 优雅关闭过程中的会话快照 |
| 系统测试 | 从 CLI 独立模式到 Daemon 模式的迁移路径 |

---

## 13. 里程碑建议

| 阶段 | 任务 | 预估 |
|---|---|---|
| **1. HTTP Server 骨架** | axum 路由 + 中间件 + 优雅关闭 | 2 天 |
| **2. AgentLoopHost** | 会话管理 + SSE 流式 handler | 2 天 |
| **3. CLI → Daemon 代理** | qx cli 支持 --daemon-url，降级当前独立模式 | 2 天 |
| **4. ACP → Daemon 代理** | qx acp 精简为协议桥 | 1.5 天 |
| **5. API Key 管理** | keyring 集成 + provider 池 + 健康检查 | 1.5 天 |
| **6. BudgetManager** | 预算追踪 + 限流 | 1 天 |
| **7. 系统服务注册** | systemd / Windows Service | 1 天 |
| **8. 集成测试** | HTTP API + 会话生命周期 + 优雅关闭 | 2 天 |
| **合计** | | **~13 天** |
