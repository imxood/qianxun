// time.rs — 2026-06-09 加: tracing-subscriber 自定义本地时间格式化.
//
// 背景: tracing-subscriber 0.3.23 内置的 `SystemTime` 格式器输出 ISO 8601 UTC
// (e.g. "2026-06-09T11:03:13.245617Z"), 不易读且时区不友好.
// tracing-subscriber 0.3.27+ 才内置 `LocalTime`, 我们用的是 0.3.23.
//
// 方案: 实现 FormatTime trait, 用 time crate 输出 "YYYY-MM-DD HH:MM:SS.mmm" 本地时间.
// time crate 已经作为传递依赖存在, 加为直接依赖无额外 cost (features: formatting + local-offset).
//
// 跨平台: local-offset 在 Windows 上用 GetSystemTimePreciseAsFileTime, Unix 用 localtime_r.

use std::fmt;
use time::format_description::FormatItem;
use time::macros::format_description;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime;

/// 固定格式: "2026-06-09 19:03:13.245" (本地时间, 毫秒精度).
/// 用 `format_description!` 宏在编译期生成, 0 运行时解析开销.
const LOCAL_TIME_FORMAT: &[FormatItem<'static>] = format_description!(
	"[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]"
);

/// tracing-subscriber 的 FormatTime 实现.
/// 输出: 写 "2026-06-09 19:03:13.245" 到 Writer (本地时间).
pub struct LocalTime;

impl FormatTime for LocalTime {
	fn format_time(&self, w: &mut Writer<'_>) -> fmt::Result {
		// Writer 实现 fmt::Write (不是 io::Write), 所以用 time::format() 返回 String 再 write!.
		// 失败 (e.g. 沙盒环境无 localtime) 时 fallback UTC, 不阻塞日志.
		let s = match time::OffsetDateTime::now_local() {
			Ok(now) => now
				.format(&LOCAL_TIME_FORMAT)
				.unwrap_or_else(|_| "????-??-?? ??:??:??.???".to_string()),
			Err(_) => time::OffsetDateTime::now_utc()
				.format(&LOCAL_TIME_FORMAT)
				.unwrap_or_else(|_| "????-??-?? ??:??:??.???".to_string()),
		};
		write!(w, "{}", s)
	}
}
