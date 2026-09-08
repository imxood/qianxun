//! 路径级单实例（Windows）：同一份 exe 只跑一份，不同路径并存。
//!
//! 为什么不用官方 tauri-plugin-single-instance：它以应用标识符为互斥
//! 身份——安装版与 dev 构建同为 `com.qianxun.desktop`，已运行的安装版
//! 常驻托盘时，`pnpm tauri dev` 起的 exe 会在 setup 阶段被判为"第二
//! 实例"静默退出，窗口永远起不来（2026-09 排障实录）。
//!
//! 身份改为 **可执行文件路径 + 标识符** 的 FNV-1a 哈希：
//! - 路径管"同一份 exe 只跑一份"：二次启动 → 唤醒已有实例亮窗后退出；
//! - 不同路径互不干扰：安装版与 `target/debug` 的 dev 版并存；
//! - 标识符仍参与身份：e2e 产物与 dev 同在 `target/debug`（同一路径），
//!   靠 `com.qianxun.e2e` 区分，跑 e2e 不会被 dev 实例挡住。
//!
//! 机制与官方插件同构（`.setup()` 阶段判定，第二实例不建任何窗口即
//! 退出）；跨进程唤醒用命名事件替代 WM_COPYDATA——回调只亮窗，不需要
//! argv，将来要做 deep-link 式参数转发再升级。

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use tauri::plugin::{Builder as PluginBuilder, TauriPlugin};
use tauri::{AppHandle, Runtime};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateMutexW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE,
    INFINITE,
};

/// FNV-1a 偏移基与质数。纯确定性哈希，不依赖 std 哈希器的跨版本稳定性
/// （互斥体名必须跨进程、跨重启一致）。
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// 唤醒回调：在第一实例的监听线程里执行。窗口方法内部自会投递到主
/// 线程，回调里直接调用是安全的。
///
/// `AppHandle<R>: Send` 需显式声明：泛型 R 的关联类型不自动继承 auto
/// trait，具体实例化（Wry）满足它。
pub fn init<R: Runtime>(on_wake: impl Fn(&AppHandle<R>) + Send + 'static) -> TauriPlugin<R>
where
    AppHandle<R>: Send + 'static,
{
    PluginBuilder::new("path-single-instance")
        .setup(move |app, _api| {
            let id = instance_id(app.config().identifier.as_str());
            let mutex_name = wide(&format!("qianxun-single-{id:016x}"));
            let wake_name = wide(&format!("qianxun-wake-{id:016x}"));

            // 互斥体持有到进程退出（不 ReleaseMutex 是有意的）：进程崩溃
            // 内核即回收，下次启动自动成为第一实例。
            // SAFETY: 名称以 NUL 结尾；属性传 null 用默认安全描述符。
            let mutex = unsafe { CreateMutexW(std::ptr::null(), 1, mutex_name.as_ptr()) };
            if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                if !mutex.is_null() {
                    // SAFETY: CreateMutexW 刚返回的合法句柄。
                    unsafe { CloseHandle(mutex) };
                }
                wake_running_instance(&wake_name);
                // 与官方插件一致：第二实例直接退场——不建窗口、静默退出。
                app.cleanup_before_exit();
                std::process::exit(0);
            }

            // 第一实例：建唤醒事件 + 常驻监听线程（随进程一起消亡）。
            // SAFETY: 同上，NUL 结尾名称 + 默认属性；自动复位事件。
            let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, wake_name.as_ptr()) };
            if event.is_null() {
                // 事件建不出来只丢"二次启动亮窗"这一件事，不阻断主流程。
                return Ok(());
            }
            let app_handle = app.clone();
            // 整体装箱后再进闭包；句柄字段只在方法内触碰——edition 2021
            // 闭包按字段路径精确捕获，闭包里写 `event.0` 会绕过包装直接
            // 捕获裸指针（!Send），写 `event.wait_forever()` 才捕获整体。
            let event = SendHandle(event);
            std::thread::spawn(move || loop {
                let woken = event.wait_forever();
                if woken != WAIT_OBJECT_0 {
                    break;
                }
                on_wake(&app_handle);
            });
            Ok(())
        })
        .build()
}

/// 互斥身份：可执行文件路径（小写归一，Windows 路径大小写不敏感）+
/// 标识符。路径取不到时退化为仅标识符——与官方插件行为对齐。
fn instance_id(identifier: &str) -> u64 {
    let exe = std::env::current_exe()
        .map(|path| path.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let hash = fnv1a(FNV_OFFSET, exe.as_bytes());
    let hash = fnv1a(hash, b"\n");
    fnv1a(hash, identifier.as_bytes())
}

fn fnv1a(seed: u64, bytes: &[u8]) -> u64 {
    let mut hash = seed;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// 通知第一实例亮窗。它可能还在启动中（事件尚未建好），短暂重试兜底
/// 竞态；彻底等不到就算了——反正它马上会自己亮出来。
fn wake_running_instance(wake_name: &[u16]) {
    for _ in 0..10 {
        // SAFETY: 名称以 NUL 结尾。
        let event = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, wake_name.as_ptr()) };
        if !event.is_null() {
            // SAFETY: OpenEventW 刚返回的合法句柄。
            unsafe {
                SetEvent(event);
                CloseHandle(event);
            }
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// 内核句柄只是进程内令牌，跨线程移动安全（内核对象无线程亲和性）；
/// 用新类型承载规避裸指针的 !Send。字段私有化：闭包捕获按字段路径
/// 精确匹配，暴露 `.0` 会让闭包绕过 `unsafe impl Send` 直捕裸指针。
struct SendHandle(HANDLE);

// SAFETY: 见类型文档——句柄无线程亲和性，且只在拥有它的进程内使用。
unsafe impl Send for SendHandle {}

impl SendHandle {
    /// 无限等待事件触发；返回值非 WAIT_OBJECT_0 即异常（句柄失效等），
    /// 监听循环以此退出。
    fn wait_forever(&self) -> u32 {
        // SAFETY: self.0 是本实例持有的合法事件句柄，等待只读。
        unsafe { WaitForSingleObject(self.0, INFINITE) }
    }
}

/// UTF-16 宽字符 + NUL 终止：Win32 API 名称入参的统一形制。
fn wide(text: &str) -> Vec<u16> {
    OsStr::new(text)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}
