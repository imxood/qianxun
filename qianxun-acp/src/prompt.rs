// ─── 提示词执行桥接 ─────────────────────────────────────
//
// session/prompt 的处理逻辑现在在 handler.rs 中实现，
// 通过 tokio::spawn 在后台执行 processing_loop。
// 此模块作为桥接逻辑的扩展点保留。
//
// 主要流程：
//   1. AcpRequestHandler::handle_session_prompt()
//   2. 创建 AcpOutputSink（绑定到共享 output_tx 通道）
//   3. 获取 session 的 conversation 和 agent_loop
//   4. 推送用户消息
//   5. tokio::spawn 执行 processing_loop
//   6. AcpOutputSink 将事件发送到 output_tx
//   7. 服务器主循环从 output_rx 接收并转发到 transport
//
// 此文件占位，核心逻辑在 handler.rs 中。
