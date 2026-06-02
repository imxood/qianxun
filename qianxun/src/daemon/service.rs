//! systemd / Windows Service 注册 (Stage 5 最小集).
//!
//! 设计目标 (见 `docs/30_子项目规划/01-daemon.md` §10):
//! - **不实际注册** (开发机注册会污染环境), 只产出配置/脚本内容
//! - 提供 `systemd_unit_template()` 返 unit file 字符串
//! - 提供 `windows_service_install_script()` 返 PowerShell 脚本字符串
//! - 实际 install/uninstall 命令留 Stage 6 实施 (本阶段仅文档 + 模板)
//!
//! Stage 6 升级方向:
//! - 用 `systemctl --user enable --now qx-daemon` 实际启用服务
//! - 用 `windows-service` crate 0.7+ 注册 Windows Service
//! - 写 PID file + flock 防止多实例
//! - 加 `qx daemon install` / `qx daemon uninstall` / `qx daemon status` 子命令

/// systemd user-level unit file 模板.
///
/// 目标路径 (用户安装时): `~/.config/systemd/user/qx-daemon.service`
///
/// 启用: `systemctl --user daemon-reload && systemctl --user enable --now qx-daemon`
/// 日志: `journalctl --user -u qx-daemon -f`
pub fn systemd_unit_template() -> &'static str {
    r#"[Unit]
Description=Qianxun Daemon - Personal AI Assistant
Documentation=https://github.com/qianxun/qianxun
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
# %h = $HOME; ExecStart 必须绝对路径
ExecStart=%h/.cargo/bin/qx daemon --port 23900
Restart=on-failure
RestartSec=5
StartLimitBurst=3
StartLimitIntervalSec=60
# 优雅关闭: systemd 发 SIGTERM, Daemon 6 步关闭 (见 docs/30_子项目规划/01-daemon.md §10.5)
KillMode=mixed
KillSignal=SIGTERM
TimeoutStopSec=30

# 环境
Environment=RUST_LOG=info
Environment=QIANXUN_HOME=%h/.qianxun
# Stage 5: token auth (Stage 6 改为 keyring)
# 留空表示 dev 模式 (auth_middleware 放行所有请求)
Environment=QIANXUN_API_KEY=

# 资源限制 (防止 OOM)
MemoryMax=2G
CPUQuota=200%

# 日志 (journald)
StandardOutput=journal
StandardError=journal
SyslogIdentifier=qx-daemon

[Install]
WantedBy=default.target
"#
}

/// Windows Service 安装 PowerShell 脚本.
///
/// 使用方法 (管理员 PowerShell):
/// ```powershell
/// # 1. 把脚本内容保存到 install-qx-daemon.ps1
/// # 2. 执行: .\install-qx-daemon.ps1
/// # 3. 启动: Start-Service QianxunDaemon
/// # 4. 停止: Stop-Service QianxunDaemon
/// # 5. 卸载: .\install-qx-daemon.ps1 -Uninstall
/// ```
///
/// 注: 实际 SCM 注册需要 `windows-service` crate 0.7+ (FFI 到 advapi32.dll),
/// 完整 dispatcher 留 Stage 6. 本脚本先写注册表项 + NSSM 占位 (NSSM 是
/// 社区标准 "service wrapper" 工具, 比手写 FFI 简单).
pub fn windows_service_install_script() -> &'static str {
    r#"# Qianxun Daemon Windows Service 安装脚本
# Stage 5 占位: 使用 NSSM (https://nssm.cc) 作为 service wrapper
# Stage 6 改用 windows-service crate 0.7+ (纯 Rust, 不依赖 NSSM)

param(
    [switch]$Uninstall
)

$ServiceName = "QianxunDaemon"
$DisplayName = "Qianxun Daemon - Personal AI Assistant"
$ExePath = "C:\Program Files\Qianxun\qx.exe"
$WorkingDir = "C:\Program Files\Qianxun"

if ($Uninstall) {
    Write-Host "Uninstalling service $ServiceName ..."
    nssm stop $ServiceName
    nssm remove $ServiceName confirm
    Write-Host "Done."
    exit 0
}

# 检查 NSSM
if (-not (Get-Command nssm -ErrorAction SilentlyContinue)) {
    Write-Error "NSSM not found. Install via: choco install nssm"
    exit 1
}

# 检查 qx.exe
if (-not (Test-Path $ExePath)) {
    Write-Error "qx.exe not found at $ExePath. Please install Qianxun first."
    exit 1
}

Write-Host "Installing service $ServiceName ..."
nssm install $ServiceName $ExePath
nssm set $ServiceName AppDirectory $WorkingDir
nssm set $ServiceName AppParameters "daemon --port 23900"
nssm set $ServiceName DisplayName $DisplayName
nssm set $ServiceName Start SERVICE_AUTO_START
nssm set $ServiceName AppStdout "$WorkingDir\logs\qx-daemon.out.log"
nssm set $ServiceName AppStderr "$WorkingDir\logs\qx-daemon.err.log"
nssm set $ServiceName AppRotateFiles 1
nssm set $ServiceName AppRotateBytes 10485760  # 10MB
nssm set $ServiceName AppRotateOnline 1

# 环境变量
nssm set $ServiceName AppEnvironmentExtra "RUST_LOG=info" "QIANXUN_HOME=C:\Users\$env:USERNAME\.qianxun"

# 优雅关闭: 给 Daemon 30s 处理完当前请求
nssm set $ServiceName AppStopMethodConsole 0
nssm set $ServiceName AppStopMethodWindow 0
nssm set $ServiceName AppStopMethodThreads 0
nssm set $ServiceName KillTimeout 30000  # 30s

# 启动
nssm start $ServiceName
Write-Host "Service installed and started. Use 'Get-Service QianxunDaemon' to check status."
"#
}

/// 返回当前平台对应的服务文件路径 (用于 `qx daemon install` 提示).
///
/// Linux: `$HOME/.config/systemd/user/qx-daemon.service`
/// Windows: `%ProgramFiles%\Qianxun\daemon\service-install.ps1`
/// 其他: 返回 `None` (Stage 5 不支持, 留 Stage 6 macOS launchd).
pub fn default_service_file_path() -> Option<String> {
    if cfg!(target_os = "linux") {
        std::env::var("HOME")
            .ok()
            .map(|home| format!("{home}/.config/systemd/user/qx-daemon.service"))
    } else if cfg!(target_os = "windows") {
        Some(r"C:\Program Files\Qianxun\daemon\service-install.ps1".to_string())
    } else {
        // macOS: 留 Stage 6 (launchd plist)
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_systemd_unit_template_includes_required_sections() {
        let t = systemd_unit_template();
        assert!(t.contains("[Unit]"), "missing [Unit] section");
        assert!(t.contains("[Service]"), "missing [Service] section");
        assert!(t.contains("[Install]"), "missing [Install] section");
        assert!(t.contains("ExecStart="), "missing ExecStart");
        assert!(t.contains("Restart=on-failure"), "missing Restart policy");
        assert!(t.contains("QIANXUN_API_KEY"), "Stage 5 token env var not declared");
        assert!(t.contains("WantedBy=default.target"), "missing [Install] target");
    }

    #[test]
    fn test_windows_service_install_script_has_install_and_uninstall() {
        let s = windows_service_install_script();
        assert!(s.contains("param("), "missing param block");
        assert!(s.contains("$Uninstall"), "missing -Uninstall switch");
        assert!(s.contains("nssm install"), "missing nssm install");
        assert!(s.contains("nssm remove"), "missing nssm remove (uninstall path)");
        assert!(s.contains("AppParameters"), "missing AppParameters");
    }

    #[test]
    fn test_default_service_file_path_non_empty_on_linux_or_windows() {
        // 我们在 Linux 或 Windows 上跑测试, 都应返 Some.
        // (cargo test 在开发机上是 win32, 但源码在 Linux/macOS 也应能编.)
        if cfg!(target_os = "linux") || cfg!(target_os = "windows") {
            assert!(
                default_service_file_path().is_some(),
                "expected service file path on Linux/Windows"
            );
        }
    }
}
