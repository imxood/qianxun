//! 远程网关（R1）：DSH 的唯一远程暴露面。
//!
//! 架构（ADR-012）：DSH 永远只听回环；远程设备（手机浏览器）经 EasyTier
//! 虚拟网访问本网关。网关终结 TLS-less HTTP + WebSocket：
//! - `/qx-gate?token=…`：唯一免鉴权入口，校验设备 token 后下发
//!   HttpOnly cookie 并 302 到 `/`；
//! - 其余全部路径要求 cookie/query 命中未吊销 token，否则 401；
//! - `/api/*`（含 SSE 流响应）与静态页流式转发到 DSH 回环 origin；
//! - `/api/remote.mux` 一条 WS 下行做双向桥（带 10 分钟空闲超时，
//!   防手机息屏导致的半开连接泄漏）。
//!
//! `remote.enabled=false`（默认）时**不绑 LAN**：网关任务照常启动，但只听
//! 回环 `127.0.0.1:port`（DSH 页 iframe 与回环 fetch 依赖它），并按周期
//! 重试绑 EasyTier 网卡。纠偏记录见 docs/18 B5——此前这里与 docs/04 都写
//! 「零监听 / 任务根本不启动」，与 `commands::sync` 的无条件启动矛盾，
//! 导致 docs/04 的验收项「关闭开关后端口扫描无监听」在回环侧必然失败。

pub mod commands;
pub mod gateway;
pub mod mobile_ui;

pub use mobile_ui::MobileUi;

/// 网关内共用的小响应构造器（移动层与桌面外壳层都从这里拿）。
pub(crate) use mobile_ui::text_response;

/// 已配对设备（settings.json remote 域持久化）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDevice {
    pub id: String,
    pub name: String,
    /// 随机 256bit token（hex）。配对二维码/URL 携带，cookie 存此值。
    pub token: String,
    /// 毫秒时间戳。
    pub created_at: i64,
    pub revoked: bool,
}

/// 历史网关默认端口（迁移基准：settings.json 里持久化的旧默认值
/// 在加载时迁移到按构建模式的新默认，用户自定义的其它端口原样保留）。
pub const LEGACY_GATEWAY_PORT: u16 = 17400;

/// 网关默认端口：release 23090 / debug 23091。回环 iframe 与 LAN 设备
/// 共用同一端口，实例内恒定（iframe 地址不变是 DSH revive 热吸收的
/// 前提）；debug 用不同端口是为了与安装版并存时互不抢端口。
pub fn default_gateway_port() -> u16 {
    if cfg!(debug_assertions) {
        23091
    } else {
        23090
    }
}

/// 远程域设置。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RemoteSettings {
    pub enabled: bool,
    /// 绑定 IP（须为本机某网卡地址；EasyTier 网卡地址即可）。
    pub bind_ip: String,
    pub port: u16,
    pub devices: Vec<RemoteDevice>,
}

impl Default for RemoteSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            bind_ip: String::new(),
            port: default_gateway_port(),
            devices: Vec::new(),
        }
    }
}
