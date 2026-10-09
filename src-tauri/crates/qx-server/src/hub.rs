//! Hub：在线实例注册表 + 抢占 + 配对/舰队 + 流路由（docs/19 §4.5、§6）。
//!
//! 服务端是不动点（§3.4）：实例（千寻）与消费端（手机/边缘）各自连接、各自保活，
//! hub 只做按请求桥接。实例连接的完整生命周期（hello 校验 → 会话 → 断线/抢占）
//! 都在 `run_instance_session`。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;

use crate::crypto;
use crate::db::{Db, InstanceRow};
use crate::governance::{GovConfig, Governance};
use crate::protocol::{Frame, InstanceMeta, Role};
use crate::transport::TunnelChannels;

/// 隧道侧最长静默（心跳 20s，60s 未见任何帧即判定断线）。
const HEARTBEAT_SILENCE: Duration = Duration::from_secs(60);
/// 等 hello 的超时。
const HELLO_TIMEOUT: Duration = Duration::from_secs(60);

struct InstanceConn {
    name: String,
    meta: InstanceMeta,
    ctrl: mpsc::Sender<Frame>,
    streams: yamux::Control,
    /// edge.auth 盖章密钥（docs/19 §4.3：转发请求时注入 X-QX-Edge）。
    edge_secret: Option<String>,
    /// 抢占击杀开关：新连接注册时唤醒旧会话退出（不依赖 yamux close 的传播时机）。
    kill: std::sync::Arc<tokio::sync::Notify>,
    /// 抢占代际：注册时分配；会话退出时只有当代代际才能注销。
    generation: u64,
}

pub struct Hub {
    db: Arc<Db>,
    gov: Arc<Governance>,
    inner: Mutex<HubInner>,
}

struct HubInner {
    online: HashMap<String, InstanceConn>,
    generation: u64,
}

/// 打开数据流的失败原因。
#[derive(Debug, PartialEq, Eq)]
pub enum OpenError {
    Offline,
    Transport(String),
}

impl Hub {
    pub fn new(db: Arc<Db>, gov_config: GovConfig) -> Arc<Self> {
        Arc::new(Self {
            db,
            gov: Governance::new(gov_config),
            inner: Mutex::new(HubInner {
                online: HashMap::new(),
                generation: 0,
            }),
        })
    }

    pub fn db(&self) -> &Arc<Db> {
        &self.db
    }

    pub fn governance(&self) -> &Arc<Governance> {
        &self.gov
    }

    /// 舰队快照：库中全部实例 + 在线状态/元信息（docs/19 v5.3）。
    pub fn fleet(&self) -> Vec<crate::protocol::FleetInstance> {
        let inner = self.inner.lock().expect("hub 锁");
        self.db
            .instances()
            .into_iter()
            .map(|row| {
                let conn = inner.online.get(&row.id);
                crate::protocol::FleetInstance {
                    id: row.id.clone(),
                    name: conn.map(|c| c.name.clone()).unwrap_or(row.name),
                    online: conn.is_some(),
                    qianxun: conn.map(|c| c.meta.qianxun.clone()),
                    dsh: conn.and_then(|c| c.meta.dsh.clone()),
                }
            })
            .collect()
    }

    /// 边缘盖章密钥（X-QX-Edge）。
    pub fn edge_secret(&self, instance: &str) -> Option<String> {
        self.inner
            .lock()
            .expect("hub 锁")
            .online
            .get(instance)
            .and_then(|conn| conn.edge_secret.clone())
    }

    /// 在线实例集合（/metrics 用）。
    pub fn online_ids(&self) -> Vec<String> {
        self.inner
            .lock()
            .expect("hub 锁")
            .online
            .keys()
            .cloned()
            .collect()
    }

    /// 向目标实例开一条 yamux 流（web 反代的数据面入口）。
    pub async fn open_stream(&self, instance: &str) -> Result<yamux::Stream, OpenError> {
        let mut control = {
            let inner = self.inner.lock().expect("hub 锁");
            inner
                .online
                .get(instance)
                .map(|conn| conn.streams.clone())
                .ok_or(OpenError::Offline)?
        };
        control
            .open_stream()
            .await
            .map_err(|cause| OpenError::Transport(cause.to_string()))
    }

    /// 广播一次配对成功（在线实例各收一条，更新设备清单副本）。
    pub fn notify_pair_granted(&self, device_id: &str, name: &str) {
        let frame = Frame::PairGranted {
            device_id: device_id.to_owned(),
            name: name.to_owned(),
        };
        let inner = self.inner.lock().expect("hub 锁");
        for conn in inner.online.values() {
            let _ = conn.ctrl.try_send(frame.clone());
        }
    }

    /// 注册（含抢占）：新连接替换旧连接——旧会话被 Notify 唤醒后自行退出（docs/19 §4.1）。
    fn register(
        &self,
        id: &str,
        name: String,
        meta: InstanceMeta,
        ctrl: mpsc::Sender<Frame>,
        streams: yamux::Control,
    ) -> (u64, std::sync::Arc<tokio::sync::Notify>) {
        let mut inner = self.inner.lock().expect("hub 锁");
        inner.generation += 1;
        let generation = inner.generation;
        let kill = std::sync::Arc::new(tokio::sync::Notify::new());
        if let Some(old) = inner.online.insert(
            id.to_owned(),
            InstanceConn {
                name,
                meta,
                ctrl,
                streams,
                edge_secret: None,
                kill: kill.clone(),
                generation,
            },
        ) {
            // 抢占三保险：击杀旧会话 + 关旧隧道 + 旧 ctrl 通道自然关闭。
            old.kill.notify_one();
            let mut old_streams = old.streams;
            tokio::spawn(async move {
                let _ = old_streams.close().await;
            });
        }
        (generation, kill)
    }

    fn set_edge_secret(&self, id: &str, secret: String) {
        let mut inner = self.inner.lock().expect("hub 锁");
        if let Some(conn) = inner.online.get_mut(id) {
            conn.edge_secret = Some(secret);
        }
    }

    fn unregister(&self, id: &str, generation: u64) {
        let mut inner = self.inner.lock().expect("hub 锁");
        if inner
            .online
            .get(id)
            .is_some_and(|conn| conn.generation == generation)
        {
            inner.online.remove(id);
        }
    }

    /// 服务端签发配对码（千寻 pair.create 或 admin CLI 均可发起；服务器级，docs/19 §4.4）。
    pub fn create_pair_offer(&self, issued_via: &str) -> Result<(String, i64), String> {
        self.db.create_pair_code(issued_via)
    }

    /// 兑换配对码 → 服务器级设备（返回明文 token，只此一次可见）。
    pub fn redeem_pair(&self, code: &str, name: &str) -> Result<(String, String), String> {
        self.db.redeem_pair_code(code).map_err(str::to_owned)?;
        let device_id = format!("dev-{}", crypto::random_hex(6));
        let token = format!("dv_{}", crypto::random_hex(32));
        self.db.add_device(&device_id, name, &token)?;
        // 回填码 → 设备，控制台的配对弹层才知道「配对成了哪台」。
        self.db.bind_pair_device(code, &device_id);
        self.db.audit("pair.grant", None, Some(&device_id), "{}");
        self.notify_pair_granted(&device_id, name);
        Ok((device_id, token))
    }
}

/// 实例会话：hello 校验 → 注册 → 控制帧/数据流循环 → 断线或被抢占退出。
///
/// 在 web.rs 的 /qx-ws 升级处 spawn；`duplex_pair` 载体下集成测试也走这一个入口。
pub async fn run_instance_session(hub: Arc<Hub>, channels: TunnelChannels) {
    let TunnelChannels {
        ctrl_tx,
        ctrl_rx,
        io,
    } = channels;
    let mut ctrl_rx = ctrl_rx;

    // 1) hello：必须是 instance 角色 + 预置表内 + 凭据正确（fail-closed）。
    let hello = tokio::select! {
        frame = ctrl_rx.recv() => frame,
        _ = tokio::time::sleep(HELLO_TIMEOUT) => None,
    };
    let Some(Frame::Hello {
        role: Role::Instance,
        instance_id,
        instance_token,
        meta,
        ..
    }) = hello
    else {
        let _ = ctrl_tx
            .send(Frame::Error {
                code: "bad_hello".into(),
                message: Some("首帧必须是 instance hello".into()),
            })
            .await;
        return;
    };
    let (Some(id), Some(token)) = (instance_id, instance_token) else {
        let _ = ctrl_tx
            .send(Frame::Error {
                code: "bad_hello".into(),
                message: Some("缺少 id/token".into()),
            })
            .await;
        return;
    };
    let row: Option<InstanceRow> = hub.db().instances().into_iter().find(|row| row.id == id);
    let Some(row) = row else {
        let _ = ctrl_tx
            .send(Frame::Error {
                code: "unknown_instance".into(),
                message: Some("实例未预置（fail-closed，无自动注册）".into()),
            })
            .await;
        return;
    };
    if !hub.db().instance_auth(&id, &token) {
        let _ = ctrl_tx
            .send(Frame::Error {
                code: "bad_token".into(),
                message: None,
            })
            .await;
        return;
    }

    // 2) 数据面：yamux（Client 模式——流由本端即边缘发起，实例侧被动接受；docs/19 §4.5）。
    //    字节流载体 = duplex（tokio io），compat 直连（见 transport.rs 教训记录）。
    let mut connection = yamux::Connection::new(
        tokio_util::compat::TokioAsyncReadCompatExt::compat(io),
        yamux::Config::default(),
        yamux::Mode::Client,
    );
    let streams = connection.control();
    let (generation, kill) = hub.register(
        &id,
        row.name.clone(),
        meta.clone().unwrap_or_default(),
        ctrl_tx.clone(),
        streams,
    );
    hub.db().touch_instance(&id);
    let _ = ctrl_tx
        .send(Frame::Welcome {
            server_version: crate::version().to_owned(),
            heartbeat_ms: 20_000,
            features: vec![crate::protocol::features::FLEET.into()],
        })
        .await;

    // 3) 会话循环。
    let mut silence = tokio::time::interval(HEARTBEAT_SILENCE);
    silence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    silence.tick().await; // interval 首个 tick 立即完成，跳过后再进入 select。
    loop {
        tokio::select! {
            // 抢占击杀：被新连接替换时立即退出。
            () = kill.notified() => break,
            frame = ctrl_rx.recv() => {
                let Some(frame) = frame else { break };
                match frame {
                    Frame::Ping => {
                        if ctrl_tx.send(Frame::Pong).await.is_err() {
                            break;
                        }
                    }
                    Frame::EdgeAuth { secret } => hub.set_edge_secret(&id, secret),
                    Frame::PairCreate => {
                        match hub.create_pair_offer(&id) {
                            Ok((code, expires_at)) => {
                                if ctrl_tx
                                    .send(Frame::PairOffered { code, expires_at })
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Err(cause) => {
                                let _ = ctrl_tx
                                    .send(Frame::Error {
                                        code: "pair_create_failed".into(),
                                        message: Some(cause),
                                    })
                                    .await;
                            }
                        }
                    }
                    Frame::DeviceList => {
                        let _ = ctrl_tx.send(Frame::DeviceSnapshot { devices: hub.db().devices() }).await;
                    }
                    Frame::DeviceRevoke { device_id } => {
                        let _ = hub.db().revoke_device(&device_id, "instance");
                        let _ = ctrl_tx.send(Frame::DeviceSnapshot { devices: hub.db().devices() }).await;
                    }
                    Frame::FleetGet => {
                        let _ = ctrl_tx.send(Frame::FleetSnapshot { instances: hub.fleet() }).await;
                    }
                    // 其余帧（device 侧/未用）忽略：向前兼容。
                    _ => {}
                }
            }
            stream = connection.next_stream() => {
                match stream {
                    Ok(Some(_unexpected)) => { /* 实例不该开入站流：yamux 自动回收 */ }
                    Ok(None) => break,
                    Err(_cause) => break,
                }
            }
            _ = silence.tick() => break,
        }
    }

    hub.unregister(&id, generation);
    hub.db().audit("instance.offline", Some(&id), None, "{}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::PairState;

    fn hub() -> Arc<Hub> {
        let db = Db::open_in_memory().expect("内存库");
        Hub::new(Arc::new(db), GovConfig::default())
    }

    #[test]
    fn fleet离线实例来自库() {
        let hub = hub();
        hub.db().add_instance("i1", "书房主机").expect("建档");
        let fleet = hub.fleet();
        assert_eq!(fleet.len(), 1);
        assert!(!fleet[0].online);
        assert_eq!(fleet[0].name, "书房主机");
    }

    #[test]
    fn 配对码签发与兑换() {
        let hub = hub();
        let (code, _) = hub.create_pair_offer("admin").expect("上架");
        let (device_id, token) = hub.redeem_pair(&code, "我的手机").expect("兑换");
        assert!(device_id.starts_with("dev-"));
        // 设备凭据带 dv_ 前缀（控制台「凭据」列要能一眼认出是哪一枚）。
        assert!(token.starts_with("dv_"), "设备 token 带 dv_ 前缀：{token}");
        assert!(hub.db().device_auth(&token).is_some());
        assert!(hub.redeem_pair(&code, "again").is_err(), "一次性");
        // 码 → 设备回填：控制台的配对弹层要能显示「配对成了哪台」。
        assert_eq!(
            hub.db().pair_state(&code),
            PairState::Used {
                device_name: Some("我的手机".to_owned())
            }
        );
    }
}
