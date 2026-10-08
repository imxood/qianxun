//! 控制协议帧（docs/19 §4）：统一端点 `/qx-ws` 上的 Text 消息。
//!
//! `v` 为协议主版本；本枚举序列化即 `{"v":2,"type":"hello",…}` 形态。
//! 载体无关（WS 文本帧 / Duplex 测试载体共用）。

use serde::{Deserialize, Serialize};

/// 协议主版本。
pub const PROTOCOL_VERSION: u32 = 2;

/// 客户端角色（docs/19 §4.5）：instance=千寻（注册自己并供会话），device=手机（单连接消费）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Instance,
    Device,
}

/// 千寻实例自述（hello.meta）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceMeta {
    pub qianxun: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dsh: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
}

/// 舰队条目：一台 PC 的在线快照。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetInstance {
    pub id: String,
    pub name: String,
    pub online: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qianxun: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dsh: Option<String>,
}

/// 已配对设备（展示用，无凭据）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub last_seen_at: i64,
    pub revoked: bool,
}

/// 控制帧。字段名 camelCase 与设计文档一致；带点的 type 用显式 rename。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum Frame {
    /// 注册（统一端点，角色在此声明）。
    Hello {
        role: Role,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instance_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instance_token: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device_token: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        meta: Option<InstanceMeta>,
    },
    /// 注册结果；features 供旧客户端降级（如 device-ws 未开则走 HTTP 投影）。
    Welcome {
        server_version: String,
        heartbeat_ms: u32,
        features: Vec<String>,
    },
    /// 心跳：千寻单侧发起（docs/19 §4.1）。
    Ping,
    Pong,
    /// 千寻→服务端：注册边缘密钥（进入隧道后立即发送；边缘在转发请求时盖章）。
    #[serde(rename = "edge.auth")]
    EdgeAuth {
        secret: String,
    },
    // ---- 舰队 ----
    #[serde(rename = "fleet.get")]
    FleetGet,
    #[serde(rename = "fleet.snapshot")]
    FleetSnapshot {
        instances: Vec<FleetInstance>,
    },
    #[serde(rename = "fleet.changed")]
    FleetChanged,
    // ---- 配对（服务端签发，服务器级；docs/19 §4.4）----
    #[serde(rename = "pair.create")]
    PairCreate,
    #[serde(rename = "pair.offered")]
    PairOffered {
        code: String,
        expires_at: i64,
    },
    #[serde(rename = "pair.granted")]
    PairGranted {
        device_id: String,
        name: String,
    },
    #[serde(rename = "pair.denied")]
    PairDenied {
        code: String,
        reason: String,
    },
    // ---- 设备治理 ----
    #[serde(rename = "device.list")]
    DeviceList,
    #[serde(rename = "device.snapshot")]
    DeviceSnapshot {
        devices: Vec<DeviceInfo>,
    },
    #[serde(rename = "device.revoke")]
    DeviceRevoke {
        device_id: String,
    },
    /// 错误（含注册失败码）。
    Error {
        code: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
}

impl Frame {
    /// 解码一条 Text 控制帧。
    pub fn decode(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// 编码为待发送的 Text 消息（不含换行，节省字节）。
    pub fn encode(&self) -> String {
        let mut value = serde_json::to_value(self).expect("帧序列化不会失败");
        if let Some(object) = value.as_object_mut() {
            object.insert("v".into(), PROTOCOL_VERSION.into());
        }
        serde_json::to_string(&value).expect("帧序列化不会失败")
    }
}

/// 服务端在 welcome 里广播的能力位。
pub mod features {
    /// 舰队查询（M1）。
    pub const FLEET: &str = "fleet";
    /// 配置同步（M2 预留）。
    pub const SYNC: &str = "sync";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 帧编解码往返() {
        let frames = vec![
            Frame::Hello {
                role: Role::Instance,
                instance_id: Some("abcd".into()),
                instance_token: Some("t".into()),
                device_token: None,
                meta: Some(InstanceMeta {
                    qianxun: "0.9.0".into(),
                    dsh: Some("0.2.1".into()),
                    hostname: Some("pc".into()),
                }),
            },
            Frame::Welcome {
                server_version: "0.1.0".into(),
                heartbeat_ms: 20_000,
                features: vec![features::FLEET.into()],
            },
            Frame::Ping,
            Frame::Pong,
            Frame::EdgeAuth {
                secret: "s3cr3t".into(),
            },
            Frame::FleetGet,
            Frame::FleetSnapshot {
                instances: vec![FleetInstance {
                    id: "i1".into(),
                    name: "书房主机".into(),
                    online: true,
                    qianxun: Some("0.9.0".into()),
                    dsh: None,
                }],
            },
            Frame::FleetChanged,
            Frame::PairCreate,
            Frame::PairOffered {
                code: "A1B2C3D4".into(),
                expires_at: 1_789_000_000,
            },
            Frame::PairGranted {
                device_id: "d1".into(),
                name: "手机".into(),
            },
            Frame::PairDenied {
                code: "X".into(),
                reason: "expired".into(),
            },
            Frame::DeviceList,
            Frame::DeviceSnapshot { devices: vec![] },
            Frame::DeviceRevoke {
                device_id: "d1".into(),
            },
            Frame::Error {
                code: "bad_token".into(),
                message: Some("凭据无效".into()),
            },
            Frame::Error {
                code: "unknown_instance".into(),
                message: None,
            },
        ];
        for frame in frames {
            let text = frame.encode();
            assert!(
                text.contains(&format!("\"v\":{PROTOCOL_VERSION}")),
                "{text}"
            );
            assert_eq!(Frame::decode(&text).expect("解码"), frame, "{text}");
        }
    }

    #[test]
    fn 点名帧的type字段() {
        assert!(Frame::PairCreate
            .encode()
            .contains("\"type\":\"pair.create\""));
        assert!(Frame::EdgeAuth { secret: "x".into() }
            .encode()
            .contains("\"type\":\"edge.auth\""));
        assert!(Frame::FleetGet.encode().contains("\"type\":\"fleet.get\""));
    }

    #[test]
    fn 未知字段向前兼容() {
        let text = r#"{"v":2,"type":"hello","role":"device","deviceToken":"t","futureField":1}"#;
        let frame = Frame::decode(text).expect("未知字段被忽略");
        assert_eq!(
            frame,
            Frame::Hello {
                role: Role::Device,
                instance_id: None,
                instance_token: None,
                device_token: Some("t".into()),
                meta: None,
            }
        );
    }
}
