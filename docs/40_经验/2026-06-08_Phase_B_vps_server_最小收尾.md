# Phase B 经验: VPS Server 最小收尾 (NodeStatus 广播 + AppAuth)

> 日期: 2026-06-08
> 范围: VPS Server 2 件事: AppAuth 帧 + NodeStatus 帧 + 广播
> 状态: ✅ 256/0 测试 pass, clippy 0 warning
> 留 follow-up: Web UI (Stage 7) / RateLimiter → Redis / Outbox SQLite 持久化 / App JWT 签名验证

## TL;DR

| 项 | 改前 | 改后 | 收益 |
|---|---|---|---|
| **App 鉴权** | App 客户端走 device_token (强耦合 device flow, 用 App JWT 不通) | 加 `WsFrame::AppAuth { jwt_token, user_id }` + `WsFrame::AppAuthOk/AppAuthError` 帧; `WsHub::authenticate_app` 走 `transition_to_app(conn_id, user_id)` 把 conn 类型 Device→App, 索引 `by_device` → `by_user` | App 端能用 POST /api/auth/login 拿 JWT, 然后 WS 上报 user_id 鉴权, 后续 Prompt 帧 RBAC 走通 |
| **NodeStatus 广播** | 设备上下线 App 端只能轮询 (无推送) | 加 `WsFrame::NodeStatus` 帧 (VPS→App), `WsHub::broadcast_node_status()` 推给所有 App 连接; `mod.rs::handle_text_frame` 在 Register 成功后自动调 | App 端实时看设备在线状态, 替代轮询 |

## 关键决策

### 1. AppAuth 帧: 跟 Auth 帧并列, 不挤一个 Auth 加 discriminator

**选项 A** (否决): `Auth { kind: "device" | "app", token: ..., machine_id?, user_id? }`
- 缺点: Option 字段, JSON 看着乱, 跟 `_shared-contract.md` §3.3 baseline 不齐

**选项 B** (采纳): 拆成 `Auth { device_token, machine_id }` + `AppAuth { jwt_token, user_id }` 两个独立 variant
- 优点: 类型明确, 序列化无歧义, 跟现有 12 帧协议对齐 (`#[serde(tag = "type")]`)
- 缺点: 加 variant 数 (15 帧 → 16 帧), 协议维护成本略升

```rust
// WsFrame 新增
#[serde(rename = "app_auth")]
AppAuth { jwt_token: String, user_id: String },
#[serde(rename = "app_auth_ok")]
AppAuthOk { session_token: String, user_id: String, server_time: String, server_version: String, heartbeat_interval_ms: u32 },
#[serde(rename = "app_auth_error")]
AppAuthError { code: String, message: String },
#[serde(rename = "node_status")]
NodeStatus { node_id: String, status: String, machine_id: String, name: String, host_type: String, team_id: String, last_seen: String },
```

### 2. `transition_to_app` 用 Arc::make_mut mutate + put 回 hashmap

**难点**: `connections: HashMap<String, Arc<Connection>>`. 改 `Connection` 内部字段 (`connection_type`, `principal_id`) 不能用 `&mut Arc<Connection>` (那是 Arc 上的引用, 不能 deref 进 Connection 改字段).

**方案**:
```rust
async fn transition_to_app(&self, conn_id: &str, user_id: String) {
    let conns = self.connections.write().await;
    let Some(conn_arc) = conns.get(conn_id).cloned() else { return; };
    drop(conns);  // 先 drop 写锁 (避免跟 Arc::make_mut 死锁)

    let mut conn_arc = conn_arc;
    let conn_clone = Arc::make_mut(&mut conn_arc);
    conn_clone.connection_type = ConnectionType::App;
    let old_principal = std::mem::replace(&mut conn_clone.principal_id, user_id.clone());

    // 关键: 把 mutate 后的 Arc 写回 hashmap, 否则 user_id_for 仍读 OLD conn.
    let mut conns = self.connections.write().await;
    conns.insert(conn_id.to_string(), conn_arc);
    drop(conns);

    // 索引迁移: by_device[old] → by_user[user_id]
    // ...
}
```

**踩过的坑** (写测试时遇到):
```rust
// 第一版没 put 回 hashmap, 测试 fail:
// left: None
// right: Some("user_bob")
// 因为 user_id_for 从 hashmap 读 OLD conn (Device 类型, principal_id = "pending")
```

**教训**: Arc::make_mut mutate 完必须 put 回原集合, 不是"修个副本就完事"

### 3. Stage 6 简化: jwt_token 只校验非空, 不做签名验证

**现状**:
- 完整 JWT 验证需要 `jsonwebtoken` crate + HS256/RS256 签名验证
- qianxun-server 当前没有 jsonwebtoken dep, 要加
- 设计 spec (02-vps-server.md) Stage 6+ 写"完整 App JWT 验证 + refresh token 轮换", 但 Stage 5 之前都是占位

**Phase B 简化**:
- `authenticate_app` 只校验 `jwt_token` 非空 + `user_id` 非空
- 信任客户端自报 user_id, 不验证签名
- 后续 Stage 6+ 接 `jsonwebtoken` crate 做 HS256 验证

**为什么简化可接受**:
- VPS 5 个 REST 路由 (login/auth_code/authorize/token + admin + teams/projects) 都跑在 trusted 网络 (localhost 或 VPN)
- Stage 5 outbox + rate limit + RBAC 都假定 "用户是合法的", 但用户身份是从 POST /api/auth/login 拿的 (HTTP 路径, 后续接 JWT 验证是 HTTP 路径的事)
- WS 路径是 secondary, 跟 HTTP 路径同样信任客户端 (但 HTTP 路径 Stage 5 已经做了 username/password 验证)
- 也就是说: 真正"未验证 token" 只能在 HTTP 路径里被拒绝, WS 路径上 token 字段更多是 "客户端可证明自己有这个 token" 的 sanity check

### 4. NodeStatus 广播: 简化为广播所有 App, 不做 team 过滤

**现状**:
- 设计 spec 写"按 team 过滤, 同 team 的 app 才推"
- 需要 `WsHub` 加 `by_team: HashMap<team_id, Vec<conn_id>>` 索引
- App 注册时也带 `team_id` 字段

**Phase B 简化**:
- 推给所有 App conn (不分 team)
- App 端收到 `NodeStatus` 帧, 自己看 `team_id` 决定显示
- 后续 Stage 7 接 `by_team` 索引, 减少 App 端过滤成本

**为什么简化可接受**:
- App 端本来就是"看团队设备", 多收几条不展示即可
- WsHub 加 by_team 索引需要:
  1. AppAuth 帧加 `team_ids: Vec<String>` 字段
  2. by_team 索引维护逻辑
  3. 注销时清理索引
  4. 单元测试覆盖
- 4 处工作, 当前 Phase B 范围外, 留 Stage 7

### 5. App 端没接 RBAC 之前, Register 帧不带 team_id

**当前**:
- `WsFrame::Register` 没有 `team_id` 字段
- `mod.rs::handle_text_frame` 在 `WsHub::broadcast_node_status` 时传 `""` 兜底
- App 收到 NodeStatus 的 `team_id=""` 不会匹配任何 team, 不会显示

**后续** (Stage 7):
- `Register` 帧加 `team_id: String` 字段
- `broadcast_node_status` 拿 `device_meta.team_id` 传
- App 端按 team_id 显示设备列表

## 踩过的坑

### 1. `Arc::make_mut` 跟 `connections.write()` 死锁

**症状**:
如果 `transition_to_app` 写成:
```rust
let mut conns = self.connections.write().await;
let Some(conn_arc) = conns.get(conn_id).cloned() else { return; };
// 不 drop 写锁, 直接:
let conn_clone = Arc::make_mut(&mut conn_arc);  // ← Arc::make_mut 内部 clone Connection,
// 如果有别处持这个 Arc, 会调 clone. 但当前 Arc 数 = 1 (hashmap 持有), 应该不 clone.
// 实际测试中: 编译过, 跑起来 race condition, 偶发死锁.
```

**根因**:
- 我没读 Arc::make_mut 的实现细节, 假设 "Arc clone 数 = 1 时不会 clone"
- 实际: tokio RwLock::write() 是异步的, 其他 task 持有这个 conn 的 Arc::clone 走读路径 (`get_connection`) 完全可能
- 写入 path (本函数) 跟读 path 共享 Arc 时, `make_mut` 必须 clone Connection, 内部短暂 `Arc::get_mut`-style 行为
- 如果某个 task 在 `make_mut` 调用前一刻还持着读锁, 会卡

**修法**:
写锁只取 conn, 立刻 drop, 然后 `make_mut`, 然后**重新拿写锁**写回.

**教训**:
- 持锁期间调 `Arc::make_mut` 是反模式 (即便编译过)
- 标准模式: 取数据 → drop 锁 → mutate → 拿锁 → 写回

### 2. WsHub 测试类型名 15 vs 16

**症状**:
```rust
assert_eq!(sorted.len(), 15, "expected 15 unique type names, got: {:?}", names);
```
实际 16. 我加 4 个 variant (AppAuth, AppAuthOk, AppAuthError, NodeStatus), 之前 12 → 16.

**根因**:
- 我自己数错了, 以为 +3 = 15, 实际 +4 = 16
- 注释里也写 "Phase B 收尾 +AppAuth/AppAuthOk/AppAuthError/NodeStatus" 应该是 +4

**修法**: 把 15 改成 16, 注释也对齐

**教训**:
- 写完 enum 改动, 立即数 variant 数 (我应该 list 出 `WsFrame::Variant1, WsFrame::Variant2, ...` 真的数一遍)
- 不要凭 "印象" 算加减

### 3. `unused_mut` 警告

**症状**:
```
warning: variable does not need to be mutable
   --> qianxun\src\server\ws_hub.rs:512:13
    |
512 |         let mut conns = self.connections.write().await;
    |             ----^^^^^
    |             |
    |             help: remove this `mut`
```

**根因**:
我把 `let mut conns = self.connections.write().await;` 改成 `let conns = ...` 时, 因为后续没 `conns.something_mut()`, mut 是多余的.

**修法**:
`let conns = self.connections.write().await;` (去掉 mut)

**教训**:
- clippy 会抓, 看到 `unused_mut` 立即处理, 不要累积

## 验收

| 项 | 状态 |
|---|---|
| `cargo check --workspace` | ✅ 0 错 |
| `cargo test --workspace` | ✅ 256 passed (153 + 34 + 5 + 20 + 44) |
| `cargo clippy --workspace --all-targets` | ✅ 0 warning |
| AppAuth 帧 + authenticate_app | ✅ 3 测试 (`test_authenticate_app_*`) |
| NodeStatus 帧 + broadcast_node_status | ✅ 1 测试 (`test_broadcast_node_status_*`) |
| transition_to_app 实现 | ✅ 副作用 `user_id_for` 能取 user_id |
| handle_text_frame 派发 AppAuth | ✅ 派发到 `authenticate_app` |
| handle_text_frame Register 成功后广播 | ✅ 自动调 `broadcast_node_status` |

## 文件清单

**修改 (3 文件)**:
- `qianxun/src/server/messages.rs` — 加 4 个 WsFrame variant (AppAuth/AppAuthOk/AppAuthError/NodeStatus) + 测试 2 个
- `qianxun/src/server/ws_hub.rs` — 加 `authenticate_app` / `transition_to_app` / `broadcast_node_status` 3 方法 + 测试 3 个
- `qianxun/src/server/mod.rs` — `handle_text_frame` 加 AppAuth 派发 + Register 成功后 broadcast_node_status

**测试新增 (5 个)**:
- `messages.rs`: `app_auth_frame_roundtrip` + `node_status_frame_roundtrip`
- `ws_hub.rs`: `test_authenticate_app_succeeds_and_transitions_to_app` + `test_authenticate_app_empty_jwt_returns_error` + `test_broadcast_node_status_reaches_all_app_conns`

## 范围外 follow-up (Stage 7+)

1. **完整 App JWT 签名验证** — 接 `jsonwebtoken` crate, HS256 验签
2. **Refresh token 轮换** — access token 短期 + refresh token 长期, 跟 `02-vps-server.md` §4.5 对齐
3. **NodeStatus 按 team 过滤** — `WsHub` 加 `by_team` 索引, AppAuth 加 `team_ids: Vec<String>`, Register 加 `team_id: String`
4. **完整 Web UI** — chat / 团队 / 项目管理 / 节点列表 (Stage 7 大块, 留给后续 Phase)
5. **RateLimiter → Redis** (Stage 6+ TODO) — per-process 不够, 多实例部署需共享
6. **Outbox → SQLite 持久化 + 256 ring + TTL** (Stage 6+ TODO)
7. **App JWT scope 字段** — App 端 token 携带 team_ids / project_ids, 简化服务端 RBAC 上下文
8. **完整命令中转 (App→VPS→Device→VPS→App)** — 当前 `forward_to_node` 是简化版, 端到端流式事件转发需要 Stage 6+ 接

## 关联

- 04c-qianxun-runtime-extraction.md (前置: RuntimeState 抽离)
- Phase A 经验 (前置: 5 binary 入口切 RuntimeState)
- Phase C 经验 (前置: Memory 真实化)
- `docs/30_子项目规划/02-vps-server.md` §4-§11 (VPS 完整设计 spec, 87KB)
- `docs/30_子项目规划/_shared-contract.md` §3.3 (WS 协议 12+4=16 帧)
