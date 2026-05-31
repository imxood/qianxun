# 千寻 VPS Server 设计

> 版本: 0.1 | 更新: 2026-05-31 | 状态: 草案
>
> VPS Server 是千寻远程控制面的核心——用户管理、节点发现、命令中转

---

### 1.1 文件结构

```
qianxun/src/server/             # VPS Server 子命令（在单二进制内）
├── mod.rs                      # 模块入口 + pub fn run()
├── config.rs                   # 服务配置解析
├── db.rs                       # SQLite + 迁移
├── auth.rs                     # 用户认证（argon2 + JWT）
├── device.rs                   # 设备授权流程（auth-code / token）
├── ws_hub.rs                   # WebSocket Hub（多连接路由）
├── admin.rs                    # 管理员 CLI 子命令
└── web/                        # Web UI 静态文件（Svelte 构建产物）
    └── index.html
```


## 1. 设计目标

### 核心理念

> **VPS 只做控制面，不存代码、不存记忆、不调 LLM。**

| 目标 | 说明 |
|---|---|
| **用户管理** | 管理员创建用户，用户登录 Web UI |
| **设备授权** | OAuth 式 Web 授权流程，绑定开发机和用户 |
| **节点发现** | 各开发机的 Daemon 通过 VPS 互相可见 |
| **命令中转** | App/Web 通过 VPS 向开发机 Daemon 发送命令 |
| **极简部署** | 1 核 + 512MB + SQLite 即可运行 |

### 非目标

- Agent 推理（在开发机本地做）
- 记忆存储（在开发机本地存）
- 文件 I/O（在开发机本地读写）
- 大规模集群编排（个人/小团队场景）

---

## 2. 架构

### 2.1 部署拓扑

```
VPS（公网可达）
└─ qx server（axum, port 23901）
    ├─ REST API（用户管理 + 设备授权）
    ├─ WebSocket（设备连接 + App 连接）
    └─ Web UI（登录 / 授权 / 管理）
         │
         │ 公网
         ├────────────────┐
         │                 │
  Windows 开发机      Linux 开发机     手机 App
  └─ qx daemon       └─ qx daemon     └─ qx app
     WS → VPS           WS → VPS        WS → VPS
```

### 2.2 VPS Server 进程结构

```
┌─────────────────────────────────────────────────────────┐
│  qx server (port 23901)                                  │
│                                                         │
│  ┌─────────────────────────────────────────────────┐    │
│  │  REST API                                        │    │
│  │  ├─ POST /api/auth/login      → 用户登录 → JWT    │    │
│  │  ├─ POST /api/device/auth-code  → 生成授权码      │    │
│  │  ├─ POST /api/device/authorize  → 确认授权        │    │
│  │  ├─ GET  /api/device/token      → 轮询设备 token  │    │
│  │  ├─ POST /api/admin/users       → 创建用户        │    │
│  │  └─ GET  /api/admin/users       → 用户列表        │    │
│  └─────────────────────────────────────────────────┘    │
│                                                         │
│  ┌─────────────────────────────────────────────────┐    │
│  │  WebSocket Hub                                  │    │
│  │  ├─ Daemon 连接（持久的 WS，来自各开发机）       │    │
│  │  ├─ App 连接（持久的 WS，来自手机/浏览器）       │    │
│  │  └─ 消息路由：App → VPS → Daemon（命令转发）     │    │
│  └─────────────────────────────────────────────────┘    │
│                                                         │
│  ┌─────────────────────────────────────────────────┐    │
│  │  Web UI (Svelte + Vite, /_ui/)                 │    │
│  │  ├─ /login           登录页                     │    │
│  │  ├─ /authorize       授权页                     │    │
│  │  ├─ /admin/users     用户管理（管理员）          │    │
│  │  └─ /dashboard       节点总览                    │    │
│  └─────────────────────────────────────────────────┘    │
│                                                         │
│  ┌─────────────────────────────────────────────────┐    │
│  │  SQLite                                           │    │
│  │  ├─ users（用户账号）                              │    │
│  │  ├─ devices（设备注册）                            │    │
│  │  └─ auth_codes（授权码）                           │    │
│  └─────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────┘
```

---

## 3. 数据库设计

### 3.1 SQLite 表定义

```sql
-- === users 用户表
CREATE TABLE users (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,      -- argon2 hash
    role TEXT NOT NULL DEFAULT 'user',  -- 'admin' | 'user'
    created_at TEXT NOT NULL,
    disabled INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_users_username ON users(username);

-- === devices 设备表
CREATE TABLE devices (
    id TEXT PRIMARY KEY,
    host_id TEXT NOT NULL,            -- "windows-pc"
    user_id TEXT NOT NULL REFERENCES users(id),
    token_hash TEXT NOT NULL,         -- 设备 token 的 bcrypt hash
    host_type TEXT NOT NULL,          -- "windows" | "linux" | "macos"
    node_name TEXT,                   -- 用户自定义名称
    projects TEXT NOT NULL DEFAULT '[]',   -- JSON array
    workers INTEGER NOT NULL DEFAULT 0,
    caps TEXT NOT NULL DEFAULT '[]',       -- JSON array
    status TEXT NOT NULL DEFAULT 'offline', -- 'online' | 'offline'
    last_seen TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_devices_user ON devices(user_id);
CREATE INDEX idx_devices_host ON devices(host_id);
CREATE INDEX idx_devices_status ON devices(status);

-- === auth_codes 授权码表
CREATE TABLE auth_codes (
    code TEXT PRIMARY KEY,
    device_id TEXT NOT NULL REFERENCES devices(id),
    user_id TEXT,                      -- 授权后填写
    expires_at TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'  -- 'pending' | 'authorized' | 'expired'
);
CREATE INDEX idx_auth_codes_status ON auth_codes(status);

-- === ws_connections WS 连接状态追踪
CREATE TABLE ws_connections (
    id TEXT PRIMARY KEY,
    device_id TEXT NOT NULL REFERENCES devices(id),
    user_id TEXT NOT NULL REFERENCES users(id),
    connected_at TEXT NOT NULL,
    disconnected_at TEXT
);
CREATE INDEX idx_ws_device ON ws_connections(device_id);
```

### 3.2 密码存储

使用 `argon2` crate（而非 bcrypt），因为：
- Argon2id 是 OWASP 推荐的首选密码哈希算法
- Rust `argon2` crate 纯 Rust 实现，无 C 依赖
- 抗 GPU 和 ASIC 攻击的能力强于 bcrypt

```rust
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use argon2::password_hash::SaltString;

fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default().hash_password(password.as_bytes(), &salt)?;
    Ok(hash.to_string())
}

fn verify_password(password: &str, hash: &str) -> Result<bool> {
    let parsed = PasswordHash::new(hash)?;
    Ok(Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok())
}
```

### 3.3 数据库迁移

不使用迁移框架。用嵌入式 SQL 版本号：

```sql
CREATE TABLE schema_version (
    version INTEGER NOT NULL,
    applied_at TEXT NOT NULL
);
INSERT INTO schema_version VALUES (1, datetime('now'));
```

启动时检查 `schema_version`，按需执行 `ALTER TABLE`。

---

## 4. 设备授权流程

### 4.1 完整流程

```
Dev machine (Daemon)          User (Browser)               VPS
────────────────────          ──────────────               ──────────
qx daemon auth
  │ POST /api/device/auth-code                              │
  │ body: { host_id, host_type, projects, caps }            │
  │← 200: { code: "xyz123", expires_in: 300 }              │
  │                                                         │
  │ 打印 URL:                                                │
  │ https://vps.example.com/authorize?code=xyz123           │
  │                                                         │
  │                           打开 URL                       │
  │                           Web UI 显示:                   │
  │                           "设备 windows-pc 请求授权"     │
  │                           用户点击 [授权]                │
  │                              │                          │
  │                              │ POST /api/device/authorize│
  │                              │ body: { code: "xyz123" } │
  │                              │← 200: { status: "ok" }   │
  │                              │                          │
  │ 轮询 token                                              │
  │ GET /api/device/token?code=xyz123                       │
  │← 200: { token: "dt_xxxxx", expires_in: 31536000 }      │
  │                                                         │
  │ 保存 token 到 ~/.qianxun/daemon.token                    │
  │                                                         │
  │ WS wss://vps:23901/ws?token=dt_xxxxx                     │
  │← auth_ok                                                │
```

### 4.2 授权码有效期

| 凭证 | 有效期 | 存储位置 |
|---|---|---|
| 授权码 | 5 分钟 | VPS SQLite，status=pending |
| 设备 token | 长期（可吊销） | 开发机 `~/.qianxun/daemon.token` |
| 用户 JWT | 24 小时 | 浏览器 localStorage |

### 4.3 简化路径（个人单机场景）

对于个人用户（不需要多设备管理），可以跳过完整的 OAuth 流程：

```json
// ~/.qianxun/config.json
{
  vps: {
    // 跳过授权流程，直接使用预配置的 token
    device_token: "dt_xxxxx",
    server_url: "wss://vps.example.com:23901/ws",
  }
}
```

VPS 管理员预先创建设备 token 分配给用户。

---

## 5. WebSocket 协议

### 5.1 连接类型

VPS 持有多条 WebSocket 连接，按角色分类：

| 角色 | 方向 | 连接数 | 认证方式 |
|---|---|---|---|
| **Daemon** | 开发机 → VPS | 每台开发机 1 条 | 设备 token (`dt_xxx`) |
| **App** | 手机/浏览器 → VPS | 每用户 1-N 条 | JWT (`eyJxxx`) |

### 5.2 Daemon 连接协议

```
1. 连接认证
   → {"type":"auth","token":"dt_xxxxx"}
   ← {"type":"auth_ok","device_id":"d_abc","user_id":"u_123"}

2. 能力注册（仅首次，或变化时）
   → {"type":"register","projects":["qianxun","myblog"],"caps":["read_file","terminal","git"]}

3. 状态更新
   → {"type":"status","status":"busy","workers":2}

4. App → Daemon 命令中转
   ← {"type":"command","from_app":"app_xyz","seq":1,
      "payload":{"action":"read_file","path":"C:\\dev\\README.md"}}
   → {"type":"command_result","app_id":"app_xyz","seq":1,
      "data":{"content":"# qianxun\n..."}}
```

### 5.3 App 连接协议

```
1. 连接认证
   → {"type":"auth","token":"eyJxxx"}
   ← {"type":"auth_ok","user_id":"u_123"}

2. 获取节点列表
   → {"type":"node_list"}
   ← {"type":"node_list","nodes":[
        {"host_id":"windows-pc","host_type":"windows",
         "node_name":"办公台式","projects":["qianxun"],
         "workers":2,"status":"online","last_seen":"2026-05-31T10:00:00Z"},
        {"host_id":"macbook","host_type":"macos",
         "node_name":"移动开发","projects":["qianxun","myblog"],
         "workers":1,"status":"offline","last_seen":"2026-05-30T22:00:00Z"}
      ]}

3. 发送命令
   → {"type":"command","target":"windows-pc","seq":1,
      "payload":{"action":"read_file","path":"C:\\dev\\README.md"}}
   ← (通过 Daemon 转发后)
   ← {"type":"command_result","host":"windows-pc","seq":1,
      "data":{"content":"# qianxun\n..."}}

4. 实时通知
   ← {"type":"node_status","host_id":"macbook","status":"online"}
```

### 5.4 心跳与断线处理

VPS 和所有 WS 客户端之间使用 **30 秒 ping/pong** 维持连接活性：

```
VPS —[ping]→ Daemon
Daemon —[pong]→ VPS
```

**超时策略**：60 秒内未收到任何消息（包括 pong）→ 判定设备离线：

```
1. VPS 检测到超时
2. 更新 devices.status = 'offline'，记录 last_seen
3. 在 ws_connections 表中写入 disconnected_at
4. 从 WsHub 的 daemons HashMap 中移除该连接
5. 广播 {"type":"node_status","host_id":"...","status":"offline"} 给所有 App
6. 如果该设备有 pending 命令，标记为失败（"target offline"）
```

**Daemon 重连**：检测到 WS 断开后，指数退避重连：1s / 2s / 4s / 8s（上限 60s）

```
1. 重连成功后，Daemon 重新发送 auth + register
2. VPS 验证 token → 恢复 devices.status = 'online'
3. 创建新的 ws_connections 记录
4. 广播 node_status online 通知
```

**App 连接断线**：App 侧断线只清理该 App 的 ws_connections，不影响 devices 表。

心跳包格式：

```json
// VPS → Client
→ {"type":"ping"}
// Client → VPS
← {"type":"pong"}
```

### 5.5 VPS WebSocket Hub
    apps: Arc<RwLock<HashMap<String, AppConnection>>>,
}

struct DaemonConnection {
    device_id: String,
    user_id: String,
    host_id: String,
    projects: Vec<String>,
    caps: Vec<String>,
    status: DeviceStatus,
    sender: mpsc::UnboundedSender<Message>,
}

struct AppConnection {
    user_id: String,
    sender: mpsc::UnboundedSender<Message>,
}

impl WsHub {
    /// App 向 Daemon 发送命令
    async fn send_command(
        &self, 
        app_id: &str, 
        target_host: &str, 
        payload: Value,
    ) -> Result<(), WsError> {
        let daemons = self.daemons.read().unwrap();
        let daemon = daemons.values()
            .find(|d| d.host_id == target_host)
            .ok_or(WsError::DeviceOffline)?;
        
        // 验证 app 有权限访问此设备（同 user_id）
        let app = self.apps.read().unwrap().get(app_id).ok_or(WsError::AppNotFound)?;
        if app.user_id != daemon.user_id {
            return Err(WsError::PermissionDenied);
        }
        
        daemon.sender.send(Message::Command { from_app: app_id, payload })?;
        Ok(())
    }
}
```

---

## 6. 安全模型

### 6.1 凭证链

```
┌────────────┐     ┌──────────────┐     ┌────────────────┐
│ 管理员    │     │ 用户         │     │ Daemon（开发机）│
│ Web UI    │     │ Web UI       │     │                │
│ 创建用户  │────→│ 登录获得 JWT │────→│ 授权码 → token │
│           │     │ (24h 有效)   │     │ (长期有效)     │
└────────────┘     └──────────────┘     └────────────────┘
```

### 6.2 权限模型

```
管理员:
  ├─ 创建/禁用用户
  ├─ 查看所有设备
  └─ 吊销设备 token

用户:
  ├─ 管理自己的开发机（授权/取消授权）
  ├─ 查看自己的设备列表
  └─ 向自己的设备发送命令

App:
  ├─ 只能看到自己的设备
  └─ 只能向自己的设备发命令
```

### 6.3 安全措施

| 措施 | 说明 |
|---|---|
| 密码哈希 | Argon2id，salt 随机生成 |
| 设备 token | 64 字节随机 hex，bcrypt hash 存储 |
| JWT 签名 | HMAC-SHA256，密钥由环境变量 `VPS_JWT_SECRET` 提供 |
| WebSocket 认证 | 所有 WS 消息必须在 auth 之后 |
| 速率限制 | /api/auth/login 10 次/分钟 每 IP |
| TLS | 生产环境必须配置 TLS（Let's Encrypt） |

---

## 7. API 清单

### 7.1 REST API

```
公开:
  POST   /api/auth/login                   用户登录 → JWT
  POST   /api/device/auth-code             生成授权码
  POST   /api/device/authorize             确认授权
  GET    /api/device/token                 轮询设备 token

管理员:
  POST   /api/admin/users                  创建用户
  GET    /api/admin/users                  用户列表
  DELETE /api/admin/users/:id              禁用用户

健康检查:
  GET    /api/health                       健康检查
```

### 7.2 WebSocket

```
WS     wss://vps:23901/ws                   设备/App 连接
```

### 7.3 Web UI

```
_get  /login                               登录页
  GET  /authorize?code=xxx                  授权页
  GET  /admin/users                         用户管理
  GET  /dashboard                           节点总览
```

---

## 8. 管理员 CLI

VPS 管理员可以通过命令行管理用户：

```
qx server admin create-user <username>     ← 创建用户（交互式设置密码）
qx server admin list-users                 ← 列出所有用户
qx server admin disable-user <id>          ← 禁用用户
qx server admin reset-password <username>  ← 重置密码
```

这些命令直接连接本地 SQLite 执行操作，不需要启动 HTTP Server。

---

## 9. 部署

### 9.1 系统要求

| 项 | 最低 | 建议 |
|---|---|---|
| CPU | 1 核 | 1 核 |
| 内存 | 256 MB | 512 MB - 1 GB |
| 磁盘 | 1 GB | 10 GB |
| 网络 | 公网可达 | 固定 IP 或域名 |
| 端口 | 23901 (WS) | + 443 (TLS) |
| 数据库 | SQLite（内嵌） | — |

### 9.2 安装

```bash
# Linux (systemd)
qx server install
  → 创建 /etc/systemd/system/qx-server.service
  → systemctl enable --now qx server

# 查看状态
qx server status
qx server logs

# 启动/停止
qx server start
qx server stop
```

### 9.3 反向代理（推荐）

生产环境建议使用 Caddy 或 Nginx 做 TLS termination：

```
# Caddyfile
vps.example.com {
    reverse_proxy 127.0.0.1:23901
}
```

---

## 10. 依赖清单

```toml
# qx server/Cargo.toml
[dependencies]
qianxun-core = { path = "../qianxun-core" }

axum = "0.8"
tower = "0.5"
tower-http = { version = "0.6", features = ["cors", "trace", "fs"] }
tokio = { workspace = true, features = ["full"] }
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
tracing = { workspace = true }
anyhow = { workspace = true }
chrono = { workspace = true, features = ["serde"] }
uuid = { workspace = true, features = ["v4"] }

# 密码哈希
argon2 = "0.5"

# JWT
jsonwebtoken = "9"

# SQLite
rusqlite = { version = "0.34", features = ["bundled"] }

# WebSocket
tokio-tungstenite = { version = "0.26", features = ["native-tls"] }
```

---

## 11. 测试策略

| 测试类型 | 覆盖 |
|---|---|
| 单元测试 | 密码哈希/验证、JWT 签发/验证、token 生成 |
| 单元测试 | WsHub 路由逻辑（App→Daemon 转发权限检查） |
| 集成测试 | 完整的授权流程（创建用户 → 登录 → 授权码 → token → WS 认证） |
| 集成测试 | WebSocket 多连接并发（多 Daemon + 多 App 同时在线） |
| 容错测试 | Daemon 断线重连、token 过期处理 |
| 安全测试 | SQL 注入防护、XSS 防护（Web UI） |

### 11.1 测试脚本（授权流程端到端测试）

```bash
#!/bin/bash
# 测试授权流程

# 1. 管理员创建用户
qx server admin create-user testuser --password secret123

# 2. 设备请求授权码
curl -X POST http://localhost:23901/api/device/auth-code \
  -H "Content-Type: application/json" \
  -d '{"host_id":"test-pc","host_type":"linux","projects":["qianxun"],"caps":["read_file"]}'
# → {"code":"xyz123"}

# 3. 用户确认授权（模拟 Web UI）
curl -X POST http://localhost:23901/api/device/authorize \
  -H "Content-Type: application/json" \
  -d '{"code":"xyz123"}'
# → {"status":"ok"}

# 4. 设备轮询 token
curl "http://localhost:23901/api/device/token?code=xyz123"
# → {"token":"dt_xxxxx"}
```

---

## 12. 里程碑建议

| 阶段 | 任务 | 预估 |
|---|---|---|
| **1. 数据库 + 用户管理** | SQLite 表 + argon2 密码 + CRUD API | 2 天 |
| **2. 授权流程** | 授权码 / token / JWT 全链路 | 2 天 |
| **3. WebSocket Hub** | 多连接管理 + 消息路由 + 心跳 | 3 天 |
| **4. Web UI** | Svelte 登录页/授权页/管理页 | 3 天 |
| **5. 管理员 CLI** | 用户管理子命令 | 1 天 |
| **6. 部署** | systemd 服务 + TLS 配置 | 1 天 |
| **7. 测试** | 授权流程 E2E + 多连接并发 + 安全测试 | 2 天 |
| **合计** | | **~14 天** |
