//! WebSocket 消息类型 (与 `docs/30_子项目规划/_shared-contract.md` §3.3 + `02-vps-server.md` §11.3 对齐).
//!
//! from_connection 暂未调用, 留 Phase 4.
#![allow(dead_code)]
//!
//! Stage 1 只定义 enum + serde 序列化, **不实现 handler**. Stage 2 (auth) + Stage 3 (routing)
//! 才把 `WsFrame` 派发到具体 action.

use serde::{Deserialize, Serialize};

/// WebSocket 帧.
///
/**
# 设计说明

- `type` 字段作判别式 (`#[serde(tag = "type")]`).
- 字段命名严格遵守 `_shared-contract.md` §3.3 + `02-vps-server.md` §6.4.2.
- Stage 1 不做 rate-limit / outbox / pending_command 等运行时结构 — 它们在 Stage 2-3 才引入.

# 字段命名约定

- 所有 snake_case, 严格 JSON 兼容.
- `request_id` 是 `prompt`/`event`/`event_done`/`event_error` 的关联键 (UUID v4).
- `machine_id` 来自 `register` 帧, VPS 用来做设备绑定.
*/
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsFrame {
    // ──────── Device → VPS ────────

    /// 设备鉴权握手. 来自 Daemon.
    #[serde(rename = "auth")]
    Auth {
        device_token: String,
        machine_id: String,
    },

    /// 设备注册 (auth 成功后). 上报主机能力/资源.
    #[serde(rename = "register")]
    Register {
        device_id: String,
        name: String,
        host_type: String,
        host_id: String,
        tags: Vec<String>,
        capabilities: Vec<String>,
        daemon_version: String,
        os: String,
        cpu_cores: u32,
        memory_mb: u32,
    },

    // ──────── VPS → Device ────────

    /// 鉴权成功. 下发 session_token.
    #[serde(rename = "auth_ok")]
    AuthOk {
        session_token: String,
        server_time: String,
        server_version: String,
        heartbeat_interval_ms: u32,
    },

    /// 鉴权失败. 关闭连接. (Stage 2 补齐, 与 `_shared-contract.md` §3.3 对齐.)
    #[serde(rename = "auth_error")]
    AuthError { code: String, message: String },

    /// 注册成功. 下发分配的 node_id.
    #[serde(rename = "register_ok")]
    RegisterOk { node_id: String },

    /// 注册失败. 关闭连接.
    #[serde(rename = "register_error")]
    RegisterError { code: String, message: String },

    // ──────── App → VPS → Device (命令中转) ────────

    /// App 发起 prompt, VPS 转发到目标 Device.
    ///
    /// **Stage 4 扩展**: 加 `target_project_id` 字段, 让 VPS 端可以做 RBAC 检查
    /// (`ws_hub::check_rbac`). 客户端 (App) 必须在发起 prompt 时明确指定目标
    /// project, VPS 不做 project 推断. 与 `_shared-contract.md` §3.3 baseline 的
    /// 扩展, 详见 `02-vps-server.md` §11.4 扩展清单 (Stage 4 add).
    #[serde(rename = "prompt")]
    Prompt {
        request_id: String,
        session_id: String,
        target_node_id: String,
        /// Stage 4: 目标 project, 喂给 `check_rbac` 鉴权. App 必填.
        target_project_id: String,
        messages: Vec<serde_json::Value>,
        model: String,
        max_tokens: u32,
        temperature: f32,
        stream_to_vps: bool,
        tools_enabled: bool,
        attachments: Vec<serde_json::Value>,
    },

    // ──────── Device → VPS → App (流式事件回包) ────────

    /// Device 流式事件 (text_delta / tool_use_delta / ...) 经 VPS 转发给 App.
    /// `event` 是 `_shared-contract.md` §3.2 中 12 种 SSE 事件 schema 之一.
    #[serde(rename = "event")]
    Event {
        request_id: String,
        event: serde_json::Value,
    },

    /// prompt 正常结束, 含 usage 统计.
    #[serde(rename = "event_done")]
    EventDone {
        request_id: String,
        usage: serde_json::Value,
    },

    /// prompt 异常结束. App 应停止等待.
    #[serde(rename = "event_error")]
    EventError {
        request_id: String,
        code: String,
        message: String,
    },

    // ──────── Heartbeat (双向) ────────

    /// 心跳. 双向都用同一帧.
    #[serde(rename = "heartbeat")]
    Heartbeat { ts: i64 },

    /// 心跳 ack.
    #[serde(rename = "heartbeat_ack")]
    HeartbeatAck { ts: i64 },
}

impl WsFrame {
    /// 用于 logging 的稳定判别名.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Auth { .. } => "auth",
            Self::Register { .. } => "register",
            Self::AuthOk { .. } => "auth_ok",
            Self::AuthError { .. } => "auth_error",
            Self::RegisterOk { .. } => "register_ok",
            Self::RegisterError { .. } => "register_error",
            Self::Prompt { .. } => "prompt",
            Self::Event { .. } => "event",
            Self::EventDone { .. } => "event_done",
            Self::EventError { .. } => "event_error",
            Self::Heartbeat { .. } => "heartbeat",
            Self::HeartbeatAck { .. } => "heartbeat_ack",
        }
    }

    /// 是否是 device→vps 方向 (用于 in-flight 校验, Stage 2+ 路由派发时用).
    pub fn is_device_to_vps(&self) -> bool {
        matches!(
            self,
            Self::Auth { .. }
                | Self::Register { .. }
                | Self::Event { .. }
                | Self::EventDone { .. }
                | Self::EventError { .. }
                | Self::Heartbeat { .. }
        )
    }

    /// 是否是 app→vps 方向.
    pub fn is_app_to_vps(&self) -> bool {
        matches!(self, Self::Prompt { .. } | Self::Heartbeat { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn auth_frame_roundtrip() {
        let frame = WsFrame::Auth {
            device_token: "dt_abc".into(),
            machine_id: "sha256:deadbeef".into(),
        };
        let s = serde_json::to_string(&frame).unwrap();
        assert_eq!(
            s,
            r#"{"type":"auth","device_token":"dt_abc","machine_id":"sha256:deadbeef"}"#
        );
        let back: WsFrame = serde_json::from_str(&s).unwrap();
        match back {
            WsFrame::Auth {
                device_token,
                machine_id,
            } => {
                assert_eq!(device_token, "dt_abc");
                assert_eq!(machine_id, "sha256:deadbeef");
            }
            _ => panic!("expected Auth"),
        }
    }

    #[test]
    fn register_frame_full_field_set() {
        let frame = WsFrame::Register {
            device_id: "dev_1".into(),
            name: "office-pc".into(),
            host_type: "windows".into(),
            host_id: "win-pc-01".into(),
            tags: vec!["workstation".into()],
            capabilities: vec!["chat".into(), "tools".into()],
            daemon_version: "0.3.0".into(),
            os: "windows-11-23H2".into(),
            cpu_cores: 16,
            memory_mb: 32768,
        };
        let v: serde_json::Value = serde_json::to_value(&frame).unwrap();
        assert_eq!(v["type"], "register");
        assert_eq!(v["cpu_cores"], 16);
        assert_eq!(v["memory_mb"], 32768);
        assert_eq!(v["tags"][0], "workstation");
    }

    #[test]
    fn prompt_frame_roundtrip() {
        let frame = WsFrame::Prompt {
            request_id: "req_xyz".into(),
            session_id: "sess_abc".into(),
            target_node_id: "node_001".into(),
            target_project_id: "proj_test".into(), // Stage 4 RBAC 字段 (sibling 已加, 测试 fixture 同步)
            messages: vec![json!({"role": "user", "content": "hello"})],
            model: "deepseek-v4-flash".into(),
            max_tokens: 16384,
            temperature: 0.0,
            stream_to_vps: true,
            tools_enabled: true,
            attachments: vec![],
        };
        let s = serde_json::to_string(&frame).unwrap();
        assert!(s.contains(r#""type":"prompt""#));
        assert!(s.contains(r#""stream_to_vps":true"#));
        let back: WsFrame = serde_json::from_str(&s).unwrap();
        assert!(matches!(back, WsFrame::Prompt { .. }));
    }

    #[test]
    fn type_name_covers_all_variants() {
        let frames = vec![
            WsFrame::Auth {
                device_token: "x".into(),
                machine_id: "y".into(),
            },
            WsFrame::Register {
                device_id: "d".into(),
                name: "n".into(),
                host_type: "t".into(),
                host_id: "h".into(),
                tags: vec![],
                capabilities: vec![],
                daemon_version: "0".into(),
                os: "o".into(),
                cpu_cores: 0,
                memory_mb: 0,
            },
            WsFrame::AuthOk {
                session_token: "s".into(),
                server_time: "t".into(),
                server_version: "v".into(),
                heartbeat_interval_ms: 30000,
            },
            WsFrame::AuthError {
                code: "c".into(),
                message: "m".into(),
            },
            WsFrame::RegisterOk {
                node_id: "n".into(),
            },
            WsFrame::RegisterError {
                code: "c".into(),
                message: "m".into(),
            },
            WsFrame::Prompt {
                request_id: "r".into(),
                session_id: "s".into(),
                target_node_id: "t".into(),
                target_project_id: "p".into(),
                messages: vec![],
                model: "m".into(),
                max_tokens: 0,
                temperature: 0.0,
                stream_to_vps: false,
                tools_enabled: false,
                attachments: vec![],
            },
            WsFrame::Event {
                request_id: "r".into(),
                event: json!({}),
            },
            WsFrame::EventDone {
                request_id: "r".into(),
                usage: json!({}),
            },
            WsFrame::EventError {
                request_id: "r".into(),
                code: "c".into(),
                message: "m".into(),
            },
            WsFrame::Heartbeat { ts: 0 },
            WsFrame::HeartbeatAck { ts: 0 },
        ];
        let names: Vec<&'static str> = frames.iter().map(|f| f.type_name()).collect();
        // 12 variants expected (Stage 2 补 AuthError); no duplicates.
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 12, "expected 12 unique type names, got: {:?}", names);
    }
}
