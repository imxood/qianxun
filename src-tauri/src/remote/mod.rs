//! 远程网关（R1）：DSH 的唯一远程暴露面。
//!
//! 架构（ADR-012）：DSH 永远只听回环；远程设备（手机浏览器）经 EasyTier
//! 虚拟网访问本网关。网关终结 TLS-less HTTP + WebSocket：
//! - `/qx-gate?token=…`：唯一免鉴权入口，校验设备 token 后下发
//!   HttpOnly cookie 并 302 到 `/`；
//! - 其余全部路径要求 cookie/query 命中未吊销 token，否则 401；
//! - `/api/*`（含 SSE 流响应）与静态页流式转发到 DSH 回环 origin；
//! - `/api/remote.mux` 一条 WS 下行做双向桥（带 10 分钟空闲超时，
//!   防手机息屏导致的半开连接泄漏）；
//! - `proxy`（docs/20）：可选的本地服务反代面——LAN 网卡上另绑一个端口
//!   （网关端口 + 2），整源代理给 cookie 指定的**回环**目标（默认），
//!   手机浏览器直达本机 dev server；鉴权与网关同源，目标白名单收敛。
//!
//! `remote.enabled=false`（默认）时**不绑 LAN**：网关任务照常启动，但只听
//! 回环 `127.0.0.1:port`（DSH 页 iframe 与回环 fetch 依赖它），并按周期
//! 重试绑 EasyTier 网卡。纠偏记录见 docs/18 B5——此前这里与 docs/04 都写
//! 「零监听 / 任务根本不启动」，与 `commands::sync` 的无条件启动矛盾，
//! 导致 docs/04 的验收项「关闭开关后端口扫描无监听」在回环侧必然失败。

pub mod commands;
pub mod gateway;
pub mod mobile_ui;
pub mod online;
pub mod proxy;

pub use mobile_ui::MobileUi;

/// 网关内共用的小响应构造器（移动层与桌面外壳层都从这里拿）。
pub(crate) use mobile_ui::text_response;

/// RouterHandle（docs/19 §3.3 / 评审 #2）：网关每次重建发布最新 Router，
/// 隧道端（online.rs）每条 yamux 流取当前版本服务——设备配对/吊销/DSH revive
/// 不会让隧道拿着过期快照（「撤销即时生效」的前提）。
pub fn router_handle() -> &'static arc_swap::ArcSwap<axum::Router> {
    static HANDLE: std::sync::OnceLock<arc_swap::ArcSwap<axum::Router>> =
        std::sync::OnceLock::new();
    HANDLE.get_or_init(|| arc_swap::ArcSwap::from_pointee(fallback_router()))
}

/// 边缘盖章密钥（docs/19 §4.3）：进程级随机，经隧道 edge.auth 上报给服务端；
/// 服务端转发请求时盖 X-QX-Edge，网关用它识别「经边缘鉴权的请求」。
pub fn edge_secret() -> &'static str {
    static SECRET: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SECRET.get_or_init(|| qx_server::crypto::random_hex(32))
}

/// 网关未就绪时的占位 Router（503）——隧道先于网关建立时流不至于悬空。
fn fallback_router() -> axum::Router {
    use axum::routing::any;
    axum::Router::new().fallback(any(|| async {
        (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "网关尚未就绪（等待 DSH/设置同步）",
        )
    }))
}

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
    /// 本地服务反代（docs/20）：LAN 上另绑 `port + 2`，手机直达本机
    /// 回环服务。默认开——远程访问本身就是运营者主动启用的能力面。
    pub proxy_enabled: bool,
    /// 反代目标放宽到内网（RFC1918/链路本地/ULA）。默认只允许回环。
    pub proxy_allow_lan: bool,
}

impl Default for RemoteSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            bind_ip: String::new(),
            port: default_gateway_port(),
            devices: Vec::new(),
            proxy_enabled: true,
            proxy_allow_lan: false,
        }
    }
}

/// 服务端域设置（docs/19 v5.2 §12.3）：一个地址字段，scheme 即加密语义。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ServerSettings {
    /// 服务端地址：`ws://127.0.0.1:23100`（本机/局域网）或 `wss://<域名>`（云端）。
    /// 空 = 未配置（不连接）。
    pub url: String,
    /// 实例身份（admin add-instance 预置；fail-closed）。
    pub instance_id: String,
    /// 实例凭据（32 字节 hex；**不入备份**，见 backup 过滤）。
    pub instance_token: String,
}
