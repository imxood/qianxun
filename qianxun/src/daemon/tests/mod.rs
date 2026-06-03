//! 跨 handler 集成测试 (从 daemon/ 顶层测试文件搬, 2026-06-04 Commit 13)
//!
//! 包含 3 个原 "按交付阶段" 拆分的测试文件:
//! - llm_integration_tests.rs (751) — 真实 LLM 端到端 (4 个 #[ignore])
//! - mvp1_integration_tests.rs (645) — MVP-1 prompt 端到端 (4 个)
//! - graceful_shutdown_tests (内嵌 mod.rs:395) — 优雅关闭 5 个 test
//!
//! 重组原则: 跟 persistence/tests/ + output_sink/tests/ 一致, 测试紧贴功能
//! 模块在 src 树里, 用 inline `#[cfg(test)] mod tests;` 而非 cargo tests/ 顶层
//! (因为 daemon 没 lib target).

#[cfg(test)]
mod llm_real_provider;
#[cfg(test)]
mod mvp1_prompt;
#[cfg(test)]
mod graceful_shutdown;
