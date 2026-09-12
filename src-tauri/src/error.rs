//! 统一错误。IPC 边界上的每一类失败都映射为用户可读的中文消息
//! （编码规范 §8：发生了什么 + 能做什么），禁止把英文堆栈直接给用户。

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("设置读取失败：{0}")]
    SettingsRead(String),

    #[error("设置写入失败：{0}")]
    SettingsWrite(String),

    #[error("设置内容不合法：{0}")]
    SettingsInvalid(String),

    #[error("应用数据目录不可用：{0}")]
    DataDir(String),

    #[error("托盘初始化失败：{0}")]
    Tray(String),

    #[error("进程树托管初始化失败：{0}")]
    ProcessGuard(String),

    #[error("DSH 进程启动失败：{0}")]
    Spawn(String),

    #[error("DSH 未能就绪：{0}")]
    Readiness(String),

    #[error("安装失败：{0}")]
    Install(String),

    #[error("搜索失败：{0}")]
    Search(String),

    #[error("截屏失败：{0}")]
    Screenshot(String),

    #[error("终端失败：{0}")]
    Terminal(String),

    #[error("窗口操作失败：{0}")]
    Window(String),

    #[error("笔记失败：{0}")]
    Notes(String),

    #[error("桥失败：{0}")]
    Bridge(String),

    /// 插件市场：搜索、安装/卸载、profile manifest 维护。
    #[error("插件市场：{0}")]
    Market(String),

    #[error("远程失败：{0}")]
    Remote(String),

    #[error("同步失败：{0}")]
    Sync(String),

    #[error("磁盘操作失败：{0}")]
    Disk(String),

    #[error("备份失败：{0}")]
    Backup(String),

    #[error("DSH 正在启动中，请等待")]
    AlreadyStarting,

    #[error("已有一次安装在进行中")]
    AlreadyInstalling,

    #[error("本机没有可用的 Node.js（需 {minimum} 或更高），请先安装 Node")]
    NoNodeRuntime { minimum: node_runtime::Version },

    #[error("所选 Node 没有可用的 npm，无法安装 DSH")]
    NpmMissing,

    #[error("DSH 尚未安装：请到「环境」页安装")]
    DshNotInstalled,

    /// ADR-015：千寻主版本号 ↔ DSH 适配锚点。已装的 DSH 与 `PINNED_VERSION`
    /// 不一致时禁止启动——必须由用户点「重装」装入验证过的版本。
    /// （重启 DSH 不是选项：DSH 进程在跑也意味着它在用未验证的版本。）
    #[error("DSH 版本不匹配：千寻要求 {required}，当前 {installed}。请到「环境」页点重装")]
    DshVersionMismatch { required: String, installed: String },
}

pub type Result<T> = std::result::Result<T, Error>;

/// Tauri 命令返回 Result 时要求错误可序列化。跨过 IPC 后前端拿到的
/// 就是这个字符串（见前端 AppError.toMessage 的字符串分支）。
impl serde::Serialize for Error {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
