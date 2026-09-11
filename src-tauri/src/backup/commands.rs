//! 数据备份域 IPC 命令。命令层只做「校验入参 → 调域逻辑 → 返回」，
//! 打包/解包/换入逻辑都在本文件内部的纯函数里（可直接单测，不需要
//! 起 Tauri 应用）。

use std::ffi::OsStr;
use std::fs;
use std::io::{BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::error::{Error, Result};
use crate::harness::install;
use crate::paths;

/// manifest 的 kind 标识：不是这个值的包一律拒绝。
const MANIFEST_KIND: &str = "qianxun-backup";
/// 备份包格式版本。读取时允许「小于等于当前」的旧包，拒绝更新的包。
pub const SCHEMA_VERSION: u32 = 1;

/// 包内 manifest 的固定条目名。
const MANIFEST_NAME: &str = "manifest.json";
/// 数据根里的千寻设置文件（进包条目同名）。
const SETTINGS_ENTRY: &str = "settings.json";
/// 数据根里的 DSH_HOME 目录（隔离模式，ADR-009）。
const DSH_HOME_ENTRY: &str = "dsh-home";
/// 还原回滚包目录（数据根相对）。
const BACKUPS_DIR: &str = "backups";
/// 回滚包文件名前缀。
const PRE_RESTORE_PREFIX: &str = "pre-restore-";
/// 还原时的解压暂存目录（数据根相对，还原结束即删）。
const STAGING_DIR: &str = ".restore-staging";
/// 目录遍历深度护栏：dsh-home 正常深度远小于此，超限视为异常结构。
const MAX_WALK_DEPTH: usize = 64;

// ---------------------------------------------------------------------------
// 数据形状（与前端 contract.ts 字段级一致）
// ---------------------------------------------------------------------------

/// 备份包顶层的 manifest.json；也是 backup_inspect 返回给前端的摘要。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BackupManifest {
    kind: String,
    schema_version: u32,
    /// 本地时间（展示用，排序请看文件名时间戳）。
    created_at: String,
    /// 导出时的千寻版本。
    app_version: String,
    /// 导出时的 DSH 版本（未安装 = None）。
    dsh_version: Option<String>,
    /// 导入的工作区个数（storages/workspace.json，读取失败计 0）。
    workspace_count: usize,
    /// 会话记录文件个数（dsh-home/sessions 递归计数）。
    session_count: usize,
    /// 包内数据文件总数（不含 manifest 自身）。
    file_count: u64,
}

impl BackupManifest {
    fn new(data_root: &Path, dsh_version: Option<String>, file_count: u64) -> Self {
        Self {
            kind: MANIFEST_KIND.to_owned(),
            schema_version: SCHEMA_VERSION,
            created_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            dsh_version,
            workspace_count: count_workspaces(data_root),
            session_count: count_sessions(data_root),
            file_count,
        }
    }
}

/// backup_export 返回：落盘位置与体量（前端结果条展示用）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupExportResult {
    pub path: String,
    pub size_bytes: u64,
    pub file_count: u64,
}

/// backup_restore 返回：还原统计 + 回滚包位置（None = 还原前没有可
/// 备份的现状，比如全新安装第一次还原）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupRestoreReport {
    pub restored_files: u64,
    pub pre_restore_backup: Option<String>,
}

// ---------------------------------------------------------------------------
// 收集规则
// ---------------------------------------------------------------------------

/// 单个相对路径是否排除在备份外。按路径段判断：
/// - `node_modules`：可由 pnpm 重建（DSH 插件栈）；
/// - `logs`：外壳日志，无还原价值；
/// - `*.tmp`：原子写（atomic.rs）的潜在残迹。
fn excluded(relative: &Path) -> bool {
    relative.components().any(|component| match component {
        Component::Normal(name) => {
            name == OsStr::new("node_modules")
                || name == OsStr::new("logs")
                || Path::new(name)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("tmp"))
        }
        _ => false,
    })
}

/// 收集进包文件（数据根相对路径）：`settings.json` + `dsh-home/**`。
/// 遍历时遇排除段整支剪枝（不走进 node_modules 数万级目录）。
fn collect_backup_files(data_root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if data_root.join(SETTINGS_ENTRY).is_file() {
        files.push(PathBuf::from(SETTINGS_ENTRY));
    }
    let dsh_home = data_root.join(DSH_HOME_ENTRY);
    if dsh_home.is_dir() {
        collect_dir(&dsh_home, Path::new(DSH_HOME_ENTRY), 0, &mut files)?;
    }
    files.sort();
    Ok(files)
}

fn collect_dir(dir: &Path, prefix: &Path, depth: usize, out: &mut Vec<PathBuf>) -> Result<()> {
    if depth > MAX_WALK_DEPTH {
        return Err(Error::Backup(format!(
            "目录嵌套过深，疑似异常结构：{}",
            dir.display()
        )));
    }
    let entries =
        fs::read_dir(dir).map_err(|cause| Error::Backup(format!("读取目录失败：{cause}")))?;
    for entry in entries {
        let entry = entry.map_err(|cause| Error::Backup(format!("读取目录失败：{cause}")))?;
        let relative = prefix.join(entry.file_name());
        if excluded(&relative) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_dir(&path, &relative, depth + 1, out)?;
        } else if path.is_file() {
            out.push(relative);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 打包
// ---------------------------------------------------------------------------

/// 把当前状态打包到 `target`（含 manifest）。导出与还原前的回滚包
/// 共用这一条路径，保证两边格式永远一致。
/// 返回写进包里的 manifest（调用方用于统计展示）。
fn write_archive(
    data_root: &Path,
    target: &Path,
    dsh_version: Option<String>,
) -> Result<BackupManifest> {
    let files = collect_backup_files(data_root)?;
    let manifest = BackupManifest::new(data_root, dsh_version, files.len() as u64);
    let bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|cause| Error::Backup(format!("manifest 序列化失败：{cause}")))?;

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|cause| Error::Backup(format!("创建目录失败：{cause}")))?;
    }
    let file = fs::File::create(target)
        .map_err(|cause| Error::Backup(format!("创建备份文件失败：{cause}")))?;
    let mut writer = ZipWriter::new(BufWriter::new(file));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    writer
        .start_file(MANIFEST_NAME, options)
        .map_err(|cause| Error::Backup(format!("写入备份包失败：{cause}")))?;
    writer
        .write_all(&bytes)
        .map_err(|cause| Error::Backup(format!("写入备份包失败：{cause}")))?;

    for relative in &files {
        let source = data_root.join(relative);
        // zip 条目名规范用正斜杠；Windows 反斜杠路径必须转换。
        let name = relative
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let payload = fs::read(&source)
            .map_err(|cause| Error::Backup(format!("读取 {} 失败：{cause}", relative.display())))?;
        writer
            .start_file(name, options)
            .map_err(|cause| Error::Backup(format!("写入备份包失败：{cause}")))?;
        writer
            .write_all(&payload)
            .map_err(|cause| Error::Backup(format!("写入备份包失败：{cause}")))?;
    }

    let mut writer = writer
        .finish()
        .map_err(|cause| Error::Backup(format!("收尾备份包失败：{cause}")))?;
    // 内容全部落盘后才算备份成功（finish 只是写完 zip 结构，还没 flush）。
    writer
        .flush()
        .map_err(|cause| Error::Backup(format!("备份文件落盘失败：{cause}")))?;
    let _ = writer.get_ref().sync_all();
    Ok(manifest)
}

// ---------------------------------------------------------------------------
// 检视
// ---------------------------------------------------------------------------

/// 从备份包读取并校验 manifest。不是千寻包 / 版本过新都在这里拒绝。
fn read_manifest(archive_path: &Path) -> Result<BackupManifest> {
    let file = fs::File::open(archive_path)
        .map_err(|cause| Error::Backup(format!("无法打开备份文件：{cause}")))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|cause| Error::Backup(format!("不是有效的 zip 备份包：{cause}")))?;
    let mut entry = archive
        .by_name(MANIFEST_NAME)
        .map_err(|_| Error::Backup("不是千寻备份包（缺少 manifest.json）".to_owned()))?;
    let mut text = String::new();
    entry
        .read_to_string(&mut text)
        .map_err(|cause| Error::Backup(format!("manifest 读取失败：{cause}")))?;
    let manifest: BackupManifest = serde_json::from_str(&text)
        .map_err(|cause| Error::Backup(format!("manifest 解析失败：{cause}")))?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

fn validate_manifest(manifest: &BackupManifest) -> Result<()> {
    if manifest.kind != MANIFEST_KIND {
        return Err(Error::Backup("不是千寻备份包（kind 不符）".to_owned()));
    }
    if manifest.schema_version > SCHEMA_VERSION {
        return Err(Error::Backup(format!(
            "备份包版本过新（schema {} > {}），请先升级千寻",
            manifest.schema_version, SCHEMA_VERSION
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 还原
// ---------------------------------------------------------------------------

/// 还原一个备份包到数据根。流程：检视 → 回滚包 → 解压到暂存目录 →
/// 旧数据改名让位 → 新数据就位 → 清理。任何一步失败都把错误向上抛，
/// 磁盘上始终有「回滚包 + 让位旧目录」两级保险。
fn restore_archive(archive_path: &Path, data_root: &Path) -> Result<BackupRestoreReport> {
    // 前置检视：不是合法千寻包就不动现有数据。
    read_manifest(archive_path)?;

    // 回滚包：还原前的现状固化（空数据根时跳过）。
    let pre_restore = write_pre_restore_backup(data_root)?;

    // 解压到暂存目录（先清掉可能的残迹）。
    let staging = data_root.join(STAGING_DIR);
    if staging.exists() {
        fs::remove_dir_all(&staging)
            .map_err(|cause| Error::Backup(format!("清理暂存目录失败：{cause}")))?;
    }
    let restored_files = extract_into(archive_path, &staging)?;

    // 换入。staging 无论成败都清掉；换入失败时磁盘上是「旧数据完整、
    // 新数据在回滚包里」的状态，不会半新半旧地留在正式位置。
    let swap = swap_in(&staging, data_root);
    let _ = fs::remove_dir_all(&staging);
    swap?;

    Ok(BackupRestoreReport {
        restored_files,
        pre_restore_backup: pre_restore,
    })
}

/// 把当前状态打成回滚包。返回包路径；数据根为空（全新安装）返回 None。
fn write_pre_restore_backup(data_root: &Path) -> Result<Option<String>> {
    if collect_backup_files(data_root)?.is_empty() {
        return Ok(None);
    }
    let dir = data_root.join(BACKUPS_DIR);
    let target = dir.join(format!(
        "{PRE_RESTORE_PREFIX}{}.zip",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ));
    write_archive(data_root, &target, None)?;
    prune_old_backups(&dir)?;
    Ok(Some(target.display().to_string()))
}

/// 只保留最近一份回滚包：更早的已无回滚价值，白白占空间。
fn prune_old_backups(dir: &Path) -> Result<()> {
    let mut zips: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|cause| Error::Backup(format!("读取备份目录失败：{cause}")))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(PRE_RESTORE_PREFIX))
        })
        .collect();
    zips.sort();
    while zips.len() > 1 {
        let Some(oldest) = zips.first() else {
            break;
        };
        // 删除失败不阻断还原：多留一份只是占空间。
        let _ = fs::remove_file(oldest);
        zips.remove(0);
    }
    Ok(())
}

/// 解压备份包到暂存目录。条目三重防线：zip 层 enclosed_name 防穿越、
/// 白名单防未知条目、目录条目跳过（按需创建父目录）。
/// 返回写盘的数据文件数（不含 manifest）。
fn extract_into(archive_path: &Path, staging: &Path) -> Result<u64> {
    let file = fs::File::open(archive_path)
        .map_err(|cause| Error::Backup(format!("无法打开备份文件：{cause}")))?;
    let mut archive =
        ZipArchive::new(file).map_err(|cause| Error::Backup(format!("备份包格式损坏：{cause}")))?;
    let mut count = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|cause| Error::Backup(format!("备份包读取失败：{cause}")))?;
        let Some(relative) = entry.enclosed_name().map(|path| path.to_path_buf()) else {
            return Err(Error::Backup(format!(
                "备份包内出现不安全路径：{}",
                entry.name()
            )));
        };
        if relative == Path::new(MANIFEST_NAME) {
            continue;
        }
        let allowed = relative == Path::new(SETTINGS_ENTRY) || relative.starts_with(DSH_HOME_ENTRY);
        if !allowed {
            return Err(Error::Backup(format!(
                "备份包包含未知条目：{}（可能是伪造的包）",
                relative.display()
            )));
        }
        if entry.is_dir() {
            continue;
        }
        let target = staging.join(&relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|cause| Error::Backup(format!("创建目录失败：{cause}")))?;
        }
        let mut out = fs::File::create(&target)
            .map_err(|cause| Error::Backup(format!("写入 {} 失败：{cause}", relative.display())))?;
        std::io::copy(&mut entry, &mut out)
            .map_err(|cause| Error::Backup(format!("写入 {} 失败：{cause}", relative.display())))?;
        count += 1;
    }
    Ok(count)
}

/// 把暂存目录里的数据换入数据根。Windows 的 rename 不覆盖已存在目标，
/// 所以旧数据先改名让位、新数据再就位；让位后换入失败则把旧数据改名
/// 回来（尽力回滚）。让位旧目录删除失败不影响结果（回滚包里已有完整
/// 状态），留待下次清理。
fn swap_in(staging: &Path, data_root: &Path) -> Result<()> {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    swap_one(
        &staging.join(SETTINGS_ENTRY),
        &data_root.join(SETTINGS_ENTRY),
        &data_root
            .join(SETTINGS_ENTRY)
            .with_extension(format!("json.old-{stamp}")),
    )?;
    swap_one(
        &staging.join(DSH_HOME_ENTRY),
        &data_root.join(DSH_HOME_ENTRY),
        &data_root.join(format!("{DSH_HOME_ENTRY}.old-{stamp}")),
    )?;
    Ok(())
}

/// 单个条目的「让位 → 就位 → 删旧」；staged 不存在 = 包里没有这一项，
/// 保留现状（比如旧包没有 attachments）。
fn swap_one(staged: &Path, target: &Path, aside: &Path) -> Result<()> {
    if !staged.exists() {
        return Ok(());
    }
    if target.exists() {
        fs::rename(target, aside).map_err(|cause| {
            Error::Backup(format!("旧数据让位失败（{}）：{cause}", aside.display()))
        })?;
        if let Err(cause) = fs::rename(staged, target) {
            let _ = fs::rename(aside, target);
            return Err(Error::Backup(format!(
                "新数据就位失败（{}）：{cause}（旧数据已回位）",
                target.display()
            )));
        }
        // 旧目录删除失败不阻断：占点空间，不影响正确性。
        if aside.is_dir() {
            let _ = fs::remove_dir_all(aside);
        } else {
            let _ = fs::remove_file(aside);
        }
    } else {
        fs::rename(staged, target).map_err(|cause| {
            Error::Backup(format!("新数据就位失败（{}）：{cause}", target.display()))
        })?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 统计（manifest 用，读取失败一律计 0，绝不影响备份本身）
// ---------------------------------------------------------------------------

/// 工作区个数：storages/workspace.json 的 global.workspaceIds 长度。
fn count_workspaces(data_root: &Path) -> usize {
    let Ok(text) = fs::read_to_string(
        data_root
            .join(DSH_HOME_ENTRY)
            .join("storages")
            .join("workspace.json"),
    ) else {
        return 0;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return 0;
    };
    value
        .get("global")
        .and_then(|global| global.get("workspaceIds"))
        .and_then(|ids| ids.as_array())
        .map(|ids| ids.len())
        .unwrap_or(0)
}

/// 会话文件个数：dsh-home/sessions 递归文件计数（不套排除规则——
/// sessions 下本就不该有 node_modules，数出来的就是会话记录）。
fn count_sessions(data_root: &Path) -> usize {
    fn walk(dir: &Path, depth: usize, out: &mut usize) {
        if depth > MAX_WALK_DEPTH {
            return;
        }
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, depth + 1, out);
            } else if path.is_file() {
                *out += 1;
            }
        }
    }
    let mut count = 0;
    walk(
        &data_root.join(DSH_HOME_ENTRY).join("sessions"),
        0,
        &mut count,
    );
    count
}

// ---------------------------------------------------------------------------
// IPC 命令
// ---------------------------------------------------------------------------

/// 导出备份：把当前数据根的「千寻设置 + DSH 用户数据」打包为 zip。
/// `path` 来自前端的原生保存对话框（plugin-dialog），取消时前端不会调
/// 本命令。DSH 运行中允许导出（会话是追加写，风险低），前端负责提示。
#[tauri::command]
pub async fn backup_export(app: AppHandle, path: String) -> Result<BackupExportResult> {
    let target = PathBuf::from(path.trim().to_owned());
    if path.trim().is_empty() {
        return Err(Error::Backup("备份目标路径为空".to_owned()));
    }
    let data_root = paths::data_dir(&app)?;
    let dsh_version = install::runtime_version(&paths::harness_dir(&app)?);

    // zip 打包是重 IO，放阻塞线程池，不占异步执行器（同 harness_environment）。
    let job = tauri::async_runtime::spawn_blocking(move || {
        let manifest = write_archive(&data_root, &target, dsh_version)?;
        let size_bytes = fs::metadata(&target).map(|meta| meta.len()).unwrap_or(0);
        Ok((manifest, size_bytes, target))
    });
    let (manifest, size_bytes, target) = job
        .await
        .map_err(|cause| Error::Backup(format!("备份任务没有完成：{cause}")))??;

    crate::logging::log(
        "info",
        &format!(
            "[backup] 已导出 {}（{} 个文件，{} 字节）",
            target.display(),
            manifest.file_count,
            size_bytes
        ),
    );
    Ok(BackupExportResult {
        path: target.display().to_string(),
        size_bytes,
        file_count: manifest.file_count,
    })
}

/// 检视备份包：只读 manifest，不落任何盘。前端确认框的数据源。
#[tauri::command]
pub async fn backup_inspect(path: String) -> Result<BackupManifest> {
    let archive = PathBuf::from(path.trim().to_owned());
    let job = tauri::async_runtime::spawn_blocking(move || read_manifest(&archive));
    let manifest = job
        .await
        .map_err(|cause| Error::Backup(format!("检视任务没有完成：{cause}")))??;
    Ok(manifest)
}

/// 还原备份：要求 DSH 已停止（进程持有 sessions/storages 的打开句柄，
/// 边跑边换数据必然损坏）。成功后把还原出的 settings.json 重新载入
/// AppState，UI 立即与磁盘一致；热键/托盘/网关等运行时副作用仍以重启
/// 后的完整加载为准（前端在成功后引导重启）。
#[tauri::command]
pub async fn backup_restore(app: AppHandle, path: String) -> Result<BackupRestoreReport> {
    {
        let status = app.state::<crate::AppState>().harness.supervisor.status();
        if !matches!(
            status,
            crate::harness::supervisor::Status::Stopped
                | crate::harness::supervisor::Status::Failed { .. }
        ) {
            return Err(Error::Backup(
                "DSH 正在运行：请先停止 DSH 再还原备份".to_owned(),
            ));
        }
    }
    // 双保险（同安装流程）：状态是 Stopped 后再等监督循环退出，确保没有
    // 子进程还握着 dsh-home 的句柄——Windows 上 rename 被占用目录会失败。
    {
        let state = app.state::<crate::AppState>();
        state
            .harness
            .supervisor
            .wait_until_inactive()
            .await
            .map_err(|cause| Error::Backup(format!("DSH 没有完全停止：{cause}")))?;
    }
    let archive = PathBuf::from(path.trim().to_owned());
    let data_root = paths::data_dir(&app)?;
    let job = tauri::async_runtime::spawn_blocking(move || restore_archive(&archive, &data_root));
    let report = job
        .await
        .map_err(|cause| Error::Backup(format!("还原任务没有完成：{cause}")))??;

    let fresh = crate::settings::load(&app)?;
    {
        let state = app.state::<crate::AppState>();
        let mut guard = state
            .settings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = fresh;
    }
    crate::logging::log(
        "info",
        &format!(
            "[backup] 已还原 {} 个文件（回滚包：{}）",
            report.restored_files,
            report.pre_restore_backup.as_deref().unwrap_or("无")
        ),
    );
    Ok(report)
}

// ---------------------------------------------------------------------------
// 测试：纯函数域，全部在临时目录里跑，不碰真实用户数据
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// 建一个带完整形态的假数据根：设置 + DSH 家（含 credentials、
    /// 工作区注册表、会话、node_modules、日志残迹）。
    fn scratch(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("qx-backup-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(DSH_HOME_ENTRY).join("sessions")).unwrap();
        fs::create_dir_all(root.join(DSH_HOME_ENTRY).join("storages")).unwrap();
        fs::create_dir_all(
            root.join(DSH_HOME_ENTRY)
                .join("profiles")
                .join("web")
                .join("node_modules")
                .join("dshmarket"),
        )
        .unwrap();
        fs::write(root.join(SETTINGS_ENTRY), "{\"theme\":\"dark\"}").unwrap();
        fs::write(
            root.join(DSH_HOME_ENTRY).join("settings.yaml"),
            "llm-pi-ai: {}",
        )
        .unwrap();
        fs::write(
            root.join(DSH_HOME_ENTRY).join(".credentials.yaml"),
            "refs: {KEY: abc}",
        )
        .unwrap();
        fs::write(
            root.join(DSH_HOME_ENTRY)
                .join("storages")
                .join("workspace.json"),
            r#"{"global":{"workspaceIds":["a","b"]}}"#,
        )
        .unwrap();
        fs::write(
            root.join(DSH_HOME_ENTRY).join("sessions").join("s1.jsonl"),
            "{}",
        )
        .unwrap();
        fs::write(
            root.join(DSH_HOME_ENTRY)
                .join("profiles")
                .join("web")
                .join("node_modules")
                .join("dshmarket")
                .join("index.js"),
            "// rebuildable",
        )
        .unwrap();
        root
    }

    #[test]
    fn 收集规则含设置与dsh家且排除可重建目录() {
        let root = scratch("collect");
        let files = collect_backup_files(&root).unwrap();
        let names: Vec<String> = files
            .iter()
            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
            .collect();
        assert!(names.contains(&"settings.json".to_owned()));
        assert!(names.contains(&"dsh-home/settings.yaml".to_owned()));
        assert!(names.contains(&"dsh-home/.credentials.yaml".to_owned()));
        assert!(names.contains(&"dsh-home/sessions/s1.jsonl".to_owned()));
        // node_modules 整支剪枝。
        assert!(
            !names.iter().any(|name| name.contains("node_modules")),
            "node_modules 不应进包：{names:?}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn 导出检视往返manifest一致() {
        let root = scratch("roundtrip");
        let target = root.parent().unwrap().join("backup-roundtrip.zip");
        let manifest = write_archive(&root, &target, Some("0.1.2".to_owned())).unwrap();
        assert_eq!(manifest.workspace_count, 2);
        assert_eq!(manifest.session_count, 1);
        assert_eq!(manifest.dsh_version.as_deref(), Some("0.1.2"));
        // 文件数 = 收集数：设置 + settings.yaml + credentials + workspace + 会话
        // 共 5 个（node_modules 已排除），不含 manifest 自身。
        assert_eq!(manifest.file_count, 5);

        let inspected = read_manifest(&target).unwrap();
        assert_eq!(inspected.kind, MANIFEST_KIND);
        assert_eq!(inspected.schema_version, SCHEMA_VERSION);
        assert_eq!(inspected.file_count, manifest.file_count);
        assert_eq!(inspected.workspace_count, manifest.workspace_count);
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_file(&target);
    }

    #[test]
    fn 还原覆盖旧数据并留下回滚包() {
        let root = scratch("restore");
        let archive = root.parent().unwrap().join("backup-restore.zip");
        write_archive(&root, &archive, None).unwrap();

        // 篡改现状：改设置、删会话。还原后都应被包内状态覆盖回来。
        fs::write(root.join(SETTINGS_ENTRY), "{\"theme\":\"light\"}").unwrap();
        fs::remove_file(root.join(DSH_HOME_ENTRY).join("sessions").join("s1.jsonl")).unwrap();

        let report = restore_archive(&archive, &root).unwrap();
        assert_eq!(report.restored_files, 5);
        let pre = report.pre_restore_backup.expect("应有回滚包");
        assert!(Path::new(&pre).is_file(), "回滚包应真实存在：{pre}");
        // 还原后：设置回到包内值，会话文件回来了，暂存目录已清理。
        assert_eq!(
            fs::read_to_string(root.join(SETTINGS_ENTRY)).unwrap(),
            "{\"theme\":\"dark\"}"
        );
        assert!(root
            .join(DSH_HOME_ENTRY)
            .join("sessions")
            .join("s1.jsonl")
            .is_file());
        assert!(!root.join(STAGING_DIR).exists(), "暂存目录应清理");
        // 回滚包捕获的是「篡改后」的现状（light）。
        let pre_text = {
            let file = fs::File::open(&pre).unwrap();
            let mut archive = ZipArchive::new(file).unwrap();
            let mut entry = archive.by_name(SETTINGS_ENTRY).unwrap();
            let mut text = String::new();
            entry.read_to_string(&mut text).unwrap();
            text
        };
        assert_eq!(pre_text, "{\"theme\":\"light\"}");
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_file(&archive);
    }

    #[test]
    fn 还原拒绝非千寻包与穿越路径() {
        let root = scratch("reject");
        let bad = root.parent().unwrap().join("not-a-backup.zip");
        let file = fs::File::create(&bad).unwrap();
        let mut writer = ZipWriter::new(file);
        writer
            .start_file("random.txt", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"hi").unwrap();
        writer.finish().unwrap();
        assert!(restore_archive(&bad, &root).is_err(), "缺 manifest 应拒绝");

        // 穿越：zip 层 enclosed_name 会净化，这里伪造一个带 .. 的条目名。
        let evil = root.parent().unwrap().join("evil.zip");
        let file = fs::File::create(&evil).unwrap();
        let mut writer = ZipWriter::new(file);
        writer
            .start_file("../evil.txt", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"boom").unwrap();
        writer
            .start_file(MANIFEST_NAME, SimpleFileOptions::default())
            .unwrap();
        let manifest = BackupManifest {
            kind: MANIFEST_KIND.to_owned(),
            schema_version: SCHEMA_VERSION,
            created_at: "t".to_owned(),
            app_version: "0".to_owned(),
            dsh_version: None,
            workspace_count: 0,
            session_count: 0,
            file_count: 0,
        };
        writer
            .write_all(serde_json::to_vec(&manifest).unwrap().as_slice())
            .unwrap();
        writer.finish().unwrap();
        let outcome = restore_archive(&evil, &root);
        assert!(outcome.is_err(), "穿越条目应被拒绝");
        assert!(
            !root.parent().unwrap().join("evil.txt").exists(),
            "穿越文件不应落盘"
        );
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_file(&bad);
        let _ = fs::remove_file(&evil);
    }

    #[test]
    fn 未知顶层条目被白名单拒绝() {
        let root = scratch("whitelist");
        let evil = root.parent().unwrap().join("foreign.zip");
        let file = fs::File::create(&evil).unwrap();
        let mut writer = ZipWriter::new(file);
        writer
            .start_file("other-app/settings.json", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"{}").unwrap();
        writer
            .start_file(MANIFEST_NAME, SimpleFileOptions::default())
            .unwrap();
        let manifest = BackupManifest {
            kind: MANIFEST_KIND.to_owned(),
            schema_version: SCHEMA_VERSION,
            created_at: "t".to_owned(),
            app_version: "0".to_owned(),
            dsh_version: None,
            workspace_count: 0,
            session_count: 0,
            file_count: 0,
        };
        writer
            .write_all(serde_json::to_vec(&manifest).unwrap().as_slice())
            .unwrap();
        writer.finish().unwrap();
        assert!(restore_archive(&evil, &root).is_err(), "未知条目应拒绝");
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_file(&evil);
    }

    #[test]
    fn 回滚包只保留最近一份() {
        let dir = std::env::temp_dir().join(format!("qx-backup-prune-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for name in [
            "pre-restore-1.zip",
            "pre-restore-2.zip",
            "pre-restore-3.zip",
        ] {
            fs::write(dir.join(name), b"x").unwrap();
        }
        prune_old_backups(&dir).unwrap();
        let left: Vec<_> = fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(left.len(), 1, "只留最近一份：{:?}", left.len());
        assert_eq!(left[0].file_name().to_string_lossy(), "pre-restore-3.zip");
        let _ = fs::remove_dir_all(&dir);
    }
}
