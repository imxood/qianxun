// events — emit 出去的事件名 + payload schema 收口
//
// 现在只 state_changed (daemon health state, Stage 2 mock).
// 后续 sub-task #3+ 加:
//   - session_created.rs / message_delta.rs / plan_updated.rs
//
// 不在每个 command 里散落 emit 调用, schema 收口在这里, 方便前端类型同步 (见 _shared-contract.md).

pub mod state_changed;