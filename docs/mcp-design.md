# 千寻 MCP Client 设计

> 版本: 0.2 | 更新: 2026-06-01 | 状态: 已实现
>
> MCP Client 完整：stdio 子进程 + HTTP/SSE 传输 + ServerManager（崩溃保护）+ ToolWrapper（AgentTool 适配）

---

### 1.1 文件结构

```
qianxun-core/src/mcp/       # MCP 模块（在 core 中）
├── mod.rs                  # 模块入口
├── client.rs              # McpClient（双传输）
├── server_manager.rs      # McpServerManager（生命周期）
├── tool_wrapper.rs        # McpToolWrapper（AgentTool 适配）
├── config.rs              # McpServerConfig + 配置解析
└── transport.rs            # LineFrameTransport（stdio + HTTP/SSE）
```


## 1. 设计目标

### 核心理念

> **MCP 是千寻的「外设总线」—— 不内置能力，而是链接外部世界的标准化协议。**

| 目标 | 说明 |
|---|---|
| **零外部依赖** | 不引入 MCP SDK crate，直接基于 JSON-RPC 2.0 实现 |
| **双传输** | 本地基于 stdio 子进程，远程基于 HTTP/SSE |
| **生命周期自管** | MCP 服务器的启动、健康检查、重启由运行时统一管理。Phase 3a 由 CLI standalone 直接管理子进程，Phase 4 移交 Daemon |
| **与 ToolRegistry 无缝集成** | MCP 工具与 builtin 工具对 AgentLoop 透明 |
| **安全隔离** | 每个 MCP 服务有独立的 capabilities 声明和权限边界 |

### 非目标

- 实现完整的 MCP Server 端（千寻只做 Client）
- 支持 MCP 的 prompts 和 resources 资源模型（千寻只使用 tools）
- 跨机器 MCP 代理（MCP 服务与千寻 Daemon 同机运行）

---

## 2. MCP 协议选型

### 2.1 协议版本

**选定**：MCP 2025-03-26 (Draft)，JSON-RPC 2.0 基础协议。

只需要实现以下方法：

| 方向 | 方法 | 用途 |
|---|---|---|
| 初始化阶段 | `initialize` | 握手 + 能力协商 |
| 初始化阶段 | `notifications/initialized` | 初始化完成通知 |
| 运行时 | `tools/list` | 获取工具列表 |
| 运行时 | `tools/call` | 调用工具 |
| 运行时 | `notifications/close` | 服务端关闭通知 |

### 2.2 能力协商

千寻作为 Client 声明：

```json
{
  "capabilities": {
    "tools": {}  // 仅使用 tools 能力
  },
  "clientInfo": {
    "name": "qianxun",
    "version": "0.1.0"
  }
}
```

要求 Server 声明：
```json
{
  "capabilities": {
    "tools": {}  // 必须实现
  }
}
```

Server 可以不实现 `prompts` 和 `resources`——千寻忽略它们。

---

## 3. 传输层设计

### 3.1 传输类型

```
McpTransport enum:
  Stdio {
    command: String,
    args: Vec<String>,
    env: Option<HashMap<String, String>>,
    cwd: Option<PathBuf>,
  }
  Http {
    url: String,           // SSE endpoint
    api_key: Option<String>,
    headers: HashMap<String, String>,
  }
```

### 3.2 Stdio 传输（本地 MCP 服务）

子进程生命周期由 `McpServerManager` 管理：

```
McpServerManager
├─ servers: HashMap<String, McpServerHandle>
│
├─ start(config)  →  创建子进程
│   ├─ 设置 stdin/stdout 行帧
│   ├─ 发送 initialize 请求
│   ├─ 接收 initialize 响应（超时 5s）
│   ├─ 发送 notifications/initialized
│   ├─ 首次 list_tools（缓存）
│   └─ 返回 McpServerHandle
│
├─ stop(name)     →  发送 SIGTERM（超时 3s → SIGKILL）
│
├─ restart(name)  →  stop + start
│
└─ health_check(name) →  发送 ping（超时 2s）
```

#### 子进程关键参数

| 参数 | 值 | 理由 |
|---|---|---|
| stdin/stdout | 行分隔 JSON（\n） | MCP 协议标准 |
| stderr | 重定向到日志 | MCP 服务的日志不污染通信通道 |
| 启动超时 | 5s | 大多数 MCP 服务 1-2s 内就绪 |
| 空闲超时 | 30 分钟无调用后关闭 | 节省资源（可配置） |
| 崩溃重启 | 最多 3 次/5 分钟 | 防止崩溃循环 |

#### 行帧读写

```rust
pub struct LineFrameTransport {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    pending: HashMap<RequestId, oneshot::Sender<McpResponse>>,
}
```

与 ACP 传输层共享模式：stdout 行读取、JSON 解析、基于 request ID 的 oneshot 路由。

### 3.3 HTTP 传输（远程 MCP 服务）

```
HTTP 传输流程：

1. GET {url}/sse
   → 建立 SSE 连接
   ← 服务端返回 endpoint 事件（包含 /message 地址）

2. POST {endpoint}
   Content-Type: application/json
   Body: JSON-RPC 2.0 请求
   Authorization: Bearer {api_key}  （可选）

3. 服务端响应通过 SSE 推送回来
```

| 参数 | 值 |
|---|---|
| SSE 重连 | 指数退避：1s / 2s / 4s / 8s（上限 30s） |
| POST 超时 | 30s（工具调用可能耗时较长） |
| 认证头 | 配置中指定，固定值 |

---

## 4. 工具缓存

### 4.1 缓存策略

`list_tools` 的结果缓存 60 秒（TTL），避免每次 build_request 都调一次：

```rust
pub struct McpServerHandle {
    config: McpServerConfig,
    transport: McpTransport,
    
    // 工具缓存
    tools: Arc<RwLock<CachedTools>>,
}

struct CachedTools {
    tools: Vec<McpTool>,
    cached_at: Instant,
    ttl: Duration,  // 默认 60s
}
```

### 4.2 缓存失效

| 事件 | 行为 |
|---|---|
| 首次 start() | 强制 list_tools → 填充缓存 |
| 缓存命中且 TTL 未过期 | 直接返回 |
| 缓存已过期 | 后台刷新（list_tools）+ 返回旧缓存（stale-while-revalidate） |
| 工具调用失败（unknown tool） | 清除缓存，强制 list_tools 重试一次 |
| restart() | 清除缓存 |

---

## 5. ToolRegistry 集成

### 5.1 接线图

```
ToolRegistry
├─ builtin: HashMap<String, Arc<dyn AgentTool>>         ✅ 完成
├─ mcp_tools: HashMap<String, Arc<dyn AgentTool>>       🔧 本文
└─ skill_tools: HashMap<String, Arc<dyn AgentTool>>     📋 Future
```

### 5.2 McpToolWrapper

MCP 工具不直接实现 `AgentTool` trait，而是通过一个统一包装器：

```rust
pub struct McpToolWrapper {
    server_name: String,
    tool_name: String,
    server_handle: Arc<McpServerHandle>,
    input_schema: Value,
    description: String,
}

#[async_trait]
impl AgentTool for McpToolWrapper {
    fn name(&self) -> &str {
        // 格式："{server_name}/{tool_name}"
        // 例如 "mcp-server-fs/read_file" 
        format!("{}/{}", self.server_name, self.tool_name)
    }
    
    fn description(&self) -> &str { &self.description }
    
    fn input_schema(&self) -> Value { self.input_schema.clone() }
    
    async fn execute(&self, arguments: Value) -> Result<ToolOutput, ToolError> {
        // 通过 server_handle 转发为 MCP tools/call
        self.server_handle.call_tool(&self.tool_name, arguments).await
    }
}
```

**工具名格式 `server/tool`** 的原因：

| 方案 | 评价 |
|---|---|
| **`server/tool`**（选定） | 命名空间隔离，不同 MCP 服务的同名工具不会冲突 |
| `tool` 无前缀 | ❌ `read_file` 会与 builtin 的 ReadTextFileTool 冲突 |
| `mcp_server_name.tool` | ⚠️ 点号在某些 LLM 工具调用中解析有问题 |

### 5.3 注册流程

```
CLI standalone / Daemon 启动，或配置变更
  │
  ├─ 读取 MCP 服务器配置列表（来自 config.json）
  │
  ├─ for each server:
  │     │
  │     ├─ McpServerManager::start(config)
  │     │   ├─ 创建子进程 / 建立 HTTP 连接
  │     │   ├─ 握手 + 能力验证
  │     │   └─ list_tools → 缓存工具列表
  │     │
  │     └─ for each tool:
  │           └─ McpToolWrapper { server_name, tool_name, schema }
  │               → ToolRegistry::mcp_tools.insert(name, wrapper)
  │
  └─ ✅ 所有 MCP 工具就绪
```

### 5.4 execution_async 搜索顺序

```rust
pub async fn execute_async(&self, name: &str, args: Value) -> Result<ToolOutput, ToolError> {
    // 1. 先查 builtin
    if let Some(tool) = self.builtin.get(name) {
        return tool.execute(args).await;
    }
    
    // 2. 再查 MCP
    if let Some(tool) = self.mcp_tools.get(name) {
        // MCP 工具调用可能较慢，设置 tokio::time::timeout
        return tokio::time::timeout(Duration::from_secs(60), tool.execute(args)).await
            .map_err(|_| ToolError::Timeout(name.to_string()))??;
    }
    
    // 3. 最后查 Skill（future）
    if let Some(tool) = self.skill_tools.get(name) {
        return tool.execute(args).await;
    }
    
    Err(ToolError::NotFound(name.to_string()))
}
```

搜索顺序规则：builtin 不可被覆盖，MCP 工具不能重名覆盖 builtin。Skill 工具同理。

---

## 6. 安全边界

### 6.1 capabilities 声明

每个 MCP 服务在配置中声明其允许的操作：

```json
{
  mcp_servers: [{
    name: "filesystem",
    transport: { type: "stdio", command: "npx", args: ["-y", "@modelcontextprotocol/server-filesystem", "."] },
    // 安全边界
    allowed_tools: ["read_file", "list_directory"],  // 白名单，空 = 全部允许
    allowed_paths: ["/home/user/projects"],           // 仅限此目录下操作
  }]
}
```

### 6.2 运行时安全

| 措施 | 说明 |
|---|---|
| **工具白名单** | `allowed_tools` 过滤，未列出的工具不注册 |
| **路径沙箱** | 对文件类工具校验操作路径是否在 allowed_paths 内 |
| **超时保护** | 每个工具调用 60s 超时，超时后杀死子进程 + 重启 |
| **资源限制** | stdio 子进程最大输出 10MB（超过则截断 + 警告） |
| **用户确认** | 高风险工具（terminal、write）需要用户确认（可选） |

### 6.3 崩溃循环保护

```rust
struct CrashState {
    crashes: Vec<Instant>,
    max_crashes: u32,        // 3
    window: Duration,        // 5 分钟
}

impl McpServerManager {
    fn on_crash(&mut self, name: &str) -> Result<()> {
        let state = self.crash_states.get_mut(name).unwrap();
        let now = Instant::now();
        
        // 移除 window 之外的旧记录
        state.crashes.retain(|t| now - *t < state.window);
        state.crashes.push(now);
        
        if state.crashes.len() > state.max_crashes as usize {
            // 5 分钟内崩溃超过 3 次 → 永久停用
            self.servers.remove(name);
            return Err(anyhow!("MCP server '{}' 崩溃次数过多，已停用", name));
        }
        
        // 指数退避重启
        let backoff = 2u64.pow(state.crashes.len() as u32 - 1);
        tokio::time::sleep(Duration::from_secs(backoff)).await;
        self.start(name)?;
        Ok(())
    }
}
```

---

## 7. 配置格式

MCP 服务器配置在 `~/.qianxun/config.json` 中：

```json
{
  mcp_servers: [
    {
      name: "filesystem",
      transport: {
        type: "stdio",
        command: "npx",
        args: ["-y", "@modelcontextprotocol/server-filesystem", "/workspace"],
        env: { "NODE_PATH": "/usr/local/lib/node_modules" },
      },
      allowed_tools: ["read_file", "write_file", "list_directory"],
      allowed_paths: ["/workspace"],
      auto_start: true,        // 随 Daemon 启动
      idle_timeout_min: 30,    // 30 分钟无调用后关闭
    },
    {
      name: "github",
      transport: {
        type: "http",
        url: "http://localhost:9090/sse",
        api_key: "token-xxx",
      },
      allowed_tools: [],  // 全部允许
      auto_start: true,
    }
  ]
}
```

---

## 8. McpServerConfig 数据结构

```rust
// === mcp/config.rs

pub struct McpServerConfig {
    pub name: String,
    pub transport: McpTransportConfig,
    pub allowed_tools: Vec<String>,   // 空 = 全部允许
    pub allowed_paths: Vec<String>,   // 仅对文件类工具有效
    pub auto_start: bool,
    pub idle_timeout_min: u32,        // 0 = 不自动关闭
}

pub enum McpTransportConfig {
    Stdio {
        command: String,
        args: Vec<String>,
        env: Option<HashMap<String, String>>,
        cwd: Option<PathBuf>,
    },
    Http {
        url: String,
        api_key: Option<String>,
        headers: Option<HashMap<String, String>>,
    },
}

// === mcp/client.rs

pub struct McpClient {
    pub server_name: String,
    transport: LineFrameTransport,   // stdio 用
    tools_cache: Arc<RwLock<CachedTools>>,
    crash_state: CrashState,
}

pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

// === mcp/server_manager.rs

pub struct McpServerManager {
    servers: HashMap<String, McpClient>,
    registry: Arc<RwLock<ToolRegistry>>,
    crash_states: HashMap<String, CrashState>,
}
```

---

## 9. 错误处理

| 错误场景 | 表现 | 恢复 |
|---|---|---|
| MCP Server 未启动 | `ToolError::NotFound` | Daemon 应重试 start() |
| tools/list 超时 | 缓存为空，该服务工具不可用 | 后台重试，指数退避 |
| tools/call 超时 | `ToolError::Timeout` | AgentLoop 重试或报错 |
| tools/call 失败（工具名不存在） | `ToolError::NotFound` | 清除缓存，重新 list_tools |
| 子进程崩溃 | stderr 记录，CrashState 计数 | 自动重启（有保护） |
| HTTP 连接断开 | SSE 重连机制 | 指数退避重连 |
| 工具返回值过大（>10MB） | 截断 + 警告 | 不影响 AgentLoop |

---

## 10. 依赖清单

```toml
# qianxun-core 已有
# 不需要新增 crate

# MCP 直接基于：
# - serde_json（JSON-RPC 2.0 序列化/反序列化）
# - tokio（async 子进程/HTTP 管理）
# - reqwest（HTTP/SSE 传输）
```

**核心原则**：不引入 MCP SDK。MCP 是一个 JSON-RPC 2.0 协议，增加一层 crate 封装没有实质收益，反而增加了依赖风险和版本同步成本。

---

## 11. 测试策略

| 测试类型 | 方法 | 覆盖 |
|---|---|---|
| 单元测试 | mock 子进程 stdin/stdout | `LineFrameTransport` 的读写和超时 |
| 单元测试 | mock `McpServerHandle` | `McpToolWrapper` 的 name/schema/execute |
| 集成测试 | 启动一个真实的 MCP stdio server | 完整握手 + list_tools + call_tool 链路 |
| 容错测试 | 模拟子进程崩溃 | 自动重启 + 崩溃循环保护 |
| 性能测试 | 并发 call_tool | 不阻塞其他 MCP 服务调用 |

### 11.1 Mock MCP Server（测试用）

```python
#!/usr/bin/env python3
"""测试用 MCP Server — JSON-RPC 2.0 over stdio"""
import sys, json

def handle(req):
    if req["method"] == "initialize":
        return {"capabilities": {"tools": {}}}
    elif req["method"] == "tools/list":
        return {
            "tools": [{
                "name": "echo",
                "description": "回声测试工具",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "text": {"type": "string"}
                    },
                    "required": ["text"]
                }
            }]
        }
    elif req["method"] == "tools/call":
        return {
            "content": [{
                "type": "text",
                "text": json.dumps(req["params"]["arguments"])
            }]
        }
    return {"error": {"code": -32601, "message": "Method not found"}}

for line in sys.stdin:
    req = json.loads(line.strip())
    resp = {"jsonrpc": "2.0", "id": req["id"], **handle(req)}
    sys.stdout.write(json.dumps(resp) + "\n")
    sys.stdout.flush()
```

---

## 12. 里程碑建议

| 阶段 | 任务 | 预估 |
|---|---|---|
| **1. 核心传输** | LineFrameTransport + 子进程管理 | 2 天 |
| **2. 握手 + 工具发现** | initialize + list_tools + 缓存 | 1 天 |
| **3. ToolRegistry 集成** | McpToolWrapper + execution_async 搜索 MCP | 1 天 |
| **4. HTTP/SSE 传输** | SSE 连接 + POST 调用 + 重连 | 1.5 天 |
| **5. 安全边界** | 工具白名单 + 路径沙箱 + 超时 | 1 天 |
| **6. 崩溃保护** | CrashState + 指数退避 + 永久停用 | 0.5 天 |
| **7. 集成测试** | mock server + 真实 server 测试 | 1.5 天 |
| **合计** | | **~8.5 天** |
