//! qx-server：千寻服务端（docs/19）。
//!
//! - `protocol`：统一端点 `/qx-ws` 的控制帧（角色化 hello，§4.5）
//! - `transport`：三种载体归一（axum WS / tungstenite 客户端 / Duplex 测试对，§12.2）
//! - `tunnel`：yamux 数据面 + 实例侧 serve（含 upgrades）+ Host 断言
//! - `db`：rusqlite 状态层（实例/设备/配对码/审计，只存哈希，§6）
//! - `hub`：在线注册表、抢占、配对、舰队、流路由
//! - `governance`：流量分级 + 资源三轴 + 降级序（§5）
//! - `assets`：内嵌控制台前端（docs/20 §7.4 决策 3），build.rs 生成资产表
//! - `web`：边缘 HTTP/WS（§3）——边缘鉴权、cookie 路由、反代与 101 透传

pub mod assets;
pub mod crypto;
pub mod db;
pub mod governance;
pub mod hub;
pub mod protocol;
pub mod transport;
pub mod tunnel;
pub mod web;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
