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

    #[error("DSH 安装失败：{0}")]
    Install(String),

    #[error("搜索失败：{0}")]
    Search(String),

    #[error("截屏失败：{0}")]
    Screenshot(String),

    #[error("终端失败：{0}")]
    Terminal(String),

    #[error("笔记失败：{0}")]
    Notes(String),

    #[error("桥失败：{0}")]
    Bridge(String),

    #[error("远程失败：{0}")]
    Remote(String),

    #[error("同步失败：{0}")]
    Sync(String),

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
