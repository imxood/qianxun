//! 状态层（docs/19 §6）：rusqlite + WAL + 顺序迁移。
//!
//! 规模（≤4 实例 × ≤几十设备）用一把 `Mutex<Connection>` 串行化全部读写——
//! 单条操作微秒级，锁竞争可忽略；等价于设计文档的「单写者 actor」语义，
//! 不引连接池/ORM。敏感字段只存哈希。

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

use crate::crypto::{self, constant_time_eq};
use crate::protocol::DeviceInfo;

/// 实例（一台跑千寻的 PC）的注册记录。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstanceRow {
    pub id: String,
    pub name: String,
    pub token_hash: String,
    pub created_at: i64,
    pub last_seen_at: i64,
}

/// 一条审计事件（控制台审计页与 CSV 导出共用）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditRow {
    pub id: i64,
    pub at: i64,
    pub instance_id: Option<String>,
    pub device_id: Option<String>,
    pub event: String,
    pub detail_json: String,
}

/// 一条待落库的小时桶（聚合后写入，UPSERT 累加）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetricRow {
    pub bucket: i64,
    pub instance_id: String,
    pub device_id: String,
    pub class: String,
    pub bytes: u64,
    pub rejected: u64,
    pub shed: u64,
}

/// 配对码状态（控制台弹层状态机，docs/20 §4.9）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PairState {
    Pending { expires_at: i64 },
    Used { device_name: Option<String> },
    Expired,
    Unknown,
}

/// 设备（手机等消费端）的注册记录；token 只存哈希。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceRow {
    pub id: String,
    pub name: String,
    pub token_hash: String,
    pub created_at: i64,
    pub last_seen_at: i64,
    pub revoked: bool,
    /// 最近一次带鉴权的来源 IP（控制台「来源 IP」列）。
    pub last_ip: Option<String>,
    pub credential_issued_at: Option<i64>,
    pub credential_expires_at: Option<i64>,
    /// 掩码后的凭据提示（`dv_abcd…wxyz`）；明文只在签发那刻可见。
    pub credential_hint: Option<String>,
}

/// 设备凭据有效期：30 天（预览口径「签发至 11-08」）。
pub const DEVICE_CREDENTIAL_TTL: i64 = 30 * 86400;

/// 凭据前缀 + 掩码：让运维在两台机器的凭据列表里一眼认出同一枚。
pub fn credential_hint(plain: &str) -> String {
    let head: String = plain.chars().take(6).collect();
    let tail: String = plain
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<char>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}…{tail}")
}

/// 当前 Unix 秒。控制台层算 24h 窗口时要和库里 `at` 用同一口径，
/// 所以它必须是模块级的自由函数，而不是挂在 `Db` 上的静态方法。
pub fn unix_now() -> i64 {
    now()
}

pub struct Db {
    conn: Mutex<Connection>,
}

/// 建表/迁移。`user_version` 单调推进；本函数幂等。
const MIGRATIONS: &[&str] = &[
    // v1：实例、设备（服务器级，不绑实例——docs/19 v5.3）、配对码、审计。
    "CREATE TABLE instances (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        token_hash TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        last_seen_at INTEGER NOT NULL
    );
    CREATE TABLE devices (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        token_hash TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        last_seen_at INTEGER NOT NULL,
        revoked INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE pair_codes (
        code_hash TEXT PRIMARY KEY,
        issued_via TEXT NOT NULL,
        expires_at INTEGER NOT NULL,
        used_at INTEGER
    );
    CREATE TABLE audit (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        at INTEGER NOT NULL,
        instance_id TEXT,
        device_id TEXT,
        event TEXT NOT NULL,
        detail_json TEXT
    );
    CREATE INDEX idx_devices_token ON devices(token_hash);",
    // v2：管理面用户与会话（/console）。密码 argon2id 哈希；会话 token 只存哈希。
    "CREATE TABLE users (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL UNIQUE,
        password_hash TEXT NOT NULL,
        created_at INTEGER NOT NULL
    );
    CREATE TABLE sessions (
        token_hash TEXT PRIMARY KEY,
        user_id TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        expires_at INTEGER NOT NULL,
        last_seen_at INTEGER NOT NULL,
        last_ip TEXT
    );
    CREATE INDEX idx_sessions_user ON sessions(user_id);",
    // v3：控制台数据面（docs/20 §4）。设备补来源/凭据元数据，审计可查询，
    // 流量按小时桶落库——概览的「24h」和设备行的「占配额」都从这里来。
    "ALTER TABLE devices ADD COLUMN last_ip TEXT;
    ALTER TABLE devices ADD COLUMN credential_issued_at INTEGER;
    ALTER TABLE devices ADD COLUMN credential_expires_at INTEGER;
    ALTER TABLE devices ADD COLUMN credential_hint TEXT;
    ALTER TABLE pair_codes ADD COLUMN device_id TEXT;
    CREATE INDEX idx_audit_at ON audit(at DESC);
    CREATE TABLE metrics_hourly (
        bucket INTEGER NOT NULL,
        instance_id TEXT NOT NULL,
        device_id TEXT NOT NULL,
        class TEXT NOT NULL,
        bytes INTEGER NOT NULL DEFAULT 0,
        rejected INTEGER NOT NULL DEFAULT 0,
        shed INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (bucket, instance_id, device_id, class)
    );
    CREATE INDEX idx_metrics_device ON metrics_hourly(device_id, bucket);
    CREATE INDEX idx_metrics_bucket ON metrics_hourly(bucket);",
];

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Db {
    /// 打开（或创建）磁盘库。
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        Self::init(conn)
    }

    /// 内存库（测试）。
    pub fn open_in_memory() -> Result<Self, String> {
        Self::init(Connection::open_in_memory().map_err(|e| e.to_string())?)
    }

    fn init(conn: Connection) -> Result<Self, String> {
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| e.to_string())?;
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        for (index, script) in MIGRATIONS.iter().enumerate() {
            let target = (index + 1) as i64;
            if version < target {
                conn.execute_batch(script)
                    .map_err(|e| format!("迁移 {target} 失败：{e}"))?;
                conn.pragma_update(None, "user_version", target)
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().expect("db 锁中毒")
    }

    // ---- 实例 ----

    /// 预置一个实例（fail-closed：表只能由 admin CLI 建档，docs/19 §6.3）。
    /// 返回明文 token——只在此刻可见，库中落哈希。
    pub fn add_instance(&self, id: &str, name: &str) -> Result<String, String> {
        let token = crypto::random_hex(32);
        let hash = crypto::sha256_hex(token.as_bytes());
        let now = now();
        self.lock()
            .execute(
                "INSERT INTO instances (id, name, token_hash, created_at, last_seen_at) VALUES (?1, ?2, ?3, ?4, ?4)",
                params![id, name, hash, now],
            )
            .map_err(|e| format!("实例已存在或写入失败：{e}"))?;
        self.audit(
            "instance.add",
            Some(id),
            None,
            &format!("{{\"name\":\"{name}\"}}"),
        );
        Ok(token)
    }

    pub fn instances(&self) -> Vec<InstanceRow> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare("SELECT id, name, token_hash, created_at, last_seen_at FROM instances ORDER BY created_at")
            .expect("列清单固定");
        let rows = stmt
            .query_map([], |row| {
                Ok(InstanceRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    token_hash: row.get(2)?,
                    created_at: row.get(3)?,
                    last_seen_at: row.get(4)?,
                })
            })
            .expect("列清单固定");
        rows.filter_map(Result::ok).collect()
    }

    /// 校验实例凭据（常数时间比对哈希）。
    pub fn instance_auth(&self, id: &str, token: &str) -> bool {
        let hash = crypto::sha256_hex(token.as_bytes());
        let conn = self.lock();
        let stored: Option<String> = conn
            .query_row(
                "SELECT token_hash FROM instances WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()
            .expect("列清单固定");
        drop(conn);
        stored.is_some_and(|stored| constant_time_eq(&stored, &hash))
    }

    /// 轮换实例 token，返回新明文。
    pub fn rotate_instance_token(&self, id: &str) -> Result<String, String> {
        let token = crypto::random_hex(32);
        let hash = crypto::sha256_hex(token.as_bytes());
        let changed = self
            .lock()
            .execute(
                "UPDATE instances SET token_hash = ?2 WHERE id = ?1",
                params![id, hash],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err(format!("实例 {id} 不存在"));
        }
        self.audit("instance.rotate", Some(id), None, "{}");
        Ok(token)
    }

    pub fn touch_instance(&self, id: &str) {
        let _ = self.lock().execute(
            "UPDATE instances SET last_seen_at = ?2 WHERE id = ?1",
            params![id, now()],
        );
    }

    // ---- 设备（服务器级，docs/19 v5.3）----

    pub fn add_device(&self, id: &str, name: &str, plain_token: &str) -> Result<(), String> {
        let hash = crypto::sha256_hex(plain_token.as_bytes());
        let issued = now();
        self.lock()
            .execute(
                "INSERT INTO devices (id, name, token_hash, created_at, last_seen_at, \
                 credential_issued_at, credential_expires_at, credential_hint) \
                 VALUES (?1, ?2, ?3, ?4, ?4, ?4, ?5, ?6)",
                params![
                    id,
                    name,
                    hash,
                    issued,
                    issued + DEVICE_CREDENTIAL_TTL,
                    credential_hint(plain_token)
                ],
            )
            .map_err(|e| format!("设备写入失败：{e}"))?;
        Ok(())
    }

    /// 手工导入既有设备（agent 一次性操作，docs/19 §12.5）：token 若给出则保持不变。
    pub fn import_device(&self, name: &str, plain_token: Option<&str>) -> Result<String, String> {
        let token = plain_token
            .map(str::to_owned)
            .unwrap_or_else(|| crypto::random_hex(32));
        let id = format!("dev-{}", crypto::random_hex(6));
        self.add_device(&id, name, &token)?;
        self.audit("device.import", None, Some(&id), "{}");
        Ok(token)
    }

    pub fn devices(&self) -> Vec<DeviceInfo> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare("SELECT id, name, created_at, last_seen_at, revoked FROM devices ORDER BY created_at")
            .expect("列清单固定");
        let rows = stmt
            .query_map([], |row| {
                Ok(DeviceInfo {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    created_at: row.get(2)?,
                    last_seen_at: row.get(3)?,
                    revoked: row.get::<_, i64>(4)? != 0,
                })
            })
            .expect("列清单固定");
        rows.filter_map(Result::ok).collect()
    }

    /// 凭 token 明文定位设备（边缘鉴权入口；常数时间比对哈希）。
    pub fn device_auth(&self, token: &str) -> Option<DeviceRow> {
        if token.is_empty() {
            return None;
        }
        let hash = crypto::sha256_hex(token.as_bytes());
        let conn = self.lock();
        let row = conn
            .query_row(
                "SELECT id, name, token_hash, created_at, last_seen_at, revoked, last_ip, \
                 credential_issued_at, credential_expires_at, credential_hint \
                 FROM devices WHERE token_hash = ?1",
                params![hash],
                |row| {
                    Ok(DeviceRow {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        token_hash: row.get(2)?,
                        created_at: row.get(3)?,
                        last_seen_at: row.get(4)?,
                        revoked: row.get::<_, i64>(5)? != 0,
                        last_ip: row.get(6)?,
                        credential_issued_at: row.get(7)?,
                        credential_expires_at: row.get(8)?,
                        credential_hint: row.get(9)?,
                    })
                },
            )
            .optional()
            .expect("列清单固定");
        row.filter(|device| !device.revoked)
    }

    /// 控制台用的完整设备行（含来源 IP 与凭据元数据）。
    pub fn device_rows(&self) -> Vec<DeviceRow> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare(
                "SELECT id, name, token_hash, created_at, last_seen_at, revoked, last_ip, \
                 credential_issued_at, credential_expires_at, credential_hint \
                 FROM devices ORDER BY created_at",
            )
            .expect("列清单固定");
        let rows = stmt
            .query_map([], |row| {
                Ok(DeviceRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    token_hash: row.get(2)?,
                    created_at: row.get(3)?,
                    last_seen_at: row.get(4)?,
                    revoked: row.get::<_, i64>(5)? != 0,
                    last_ip: row.get(6)?,
                    credential_issued_at: row.get(7)?,
                    credential_expires_at: row.get(8)?,
                    credential_hint: row.get(9)?,
                })
            })
            .expect("列清单固定");
        rows.filter_map(Result::ok).collect()
    }

    /// 吊销单台设备。`via` 记进审计 detail，而不是让调用方再补一条
    /// `device.console_revoke`——一个动作两条事件，审计列表读起来像出了两次事。
    pub fn revoke_device(&self, id: &str, via: &str) -> Result<(), String> {
        let changed = self
            .lock()
            .execute("UPDATE devices SET revoked = 1 WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err(format!("设备 {id} 不存在"));
        }
        self.audit(
            "device.revoke",
            None,
            Some(id),
            &format!("{{\"via\":\"{via}\"}}"),
        );
        Ok(())
    }

    pub fn touch_device(&self, id: &str) {
        let _ = self.lock().execute(
            "UPDATE devices SET last_seen_at = ?2 WHERE id = ?1",
            params![id, now()],
        );
    }

    /// 记录来源 IP。IP 没变就不写：这是反代热路径（每个请求都会走到），
    /// 「无变化 UPDATE」在 WAL 下也要写一页，收益却只有一处展示。
    pub fn touch_device_from(&self, id: &str, ip: &str) {
        let _ = self.lock().execute(
            "UPDATE devices SET last_ip = ?2 WHERE id = ?1 AND (last_ip IS NULL OR last_ip <> ?2)",
            params![id, ip],
        );
    }

    /// 撤销吊销。
    ///
    /// 铁律（docs/20 §4.4/§4.10）：吊销只翻 `revoked` 标志位，**绝不**在这里
    /// 轮换 token——一旦凭据被换掉，「撤销」在实现上就不成立了。这里的
    /// 恢复也只是把标志位清回去，被吊销设备的下一次请求即重新生效。
    pub fn restore_device(&self, id: &str) -> Result<(), String> {
        let changed = self
            .lock()
            .execute(
                "UPDATE devices SET revoked = 0 WHERE id = ?1 AND revoked = 1",
                params![id],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err(format!("设备 {id} 不存在或未被吊销"));
        }
        self.audit("device.restore", None, Some(id), "{}");
        Ok(())
    }

    /// 批量吊销（控制台「吊销全部」，不可逆）。
    pub fn revoke_all_devices(&self) -> Result<usize, String> {
        let changed = self
            .lock()
            .execute("UPDATE devices SET revoked = 1 WHERE revoked = 0", [])
            .map_err(|e| e.to_string())?;
        self.audit("device.revoke_all", None, None, "{}");
        Ok(changed)
    }

    /// 轮换设备凭据，返回明文（只此一次可见；库中仍只存哈希）。
    pub fn rotate_device_token(&self, id: &str) -> Result<(String, i64, i64), String> {
        let token = format!("dv_{}", crypto::random_hex(32));
        let issued = now();
        let expires = issued + DEVICE_CREDENTIAL_TTL;
        let changed = self
            .lock()
            .execute(
                "UPDATE devices SET token_hash = ?2, credential_issued_at = ?3, \
                 credential_expires_at = ?4, credential_hint = ?5 WHERE id = ?1",
                params![
                    id,
                    crypto::sha256_hex(token.as_bytes()),
                    issued,
                    expires,
                    credential_hint(&token)
                ],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err(format!("设备 {id} 不存在"));
        }
        self.audit("device.rotate", None, Some(id), "{}");
        Ok((token, issued, expires))
    }

    // ---- 计量（小时桶；docs/20 §4.3） ----

    /// 批量写桶。单事务，避免每条一次 fsync。
    pub fn flush_metrics(&self, rows: &[MetricRow]) -> Result<usize, String> {
        if rows.is_empty() {
            return Ok(0);
        }
        let mut conn = self.lock();
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO metrics_hourly (bucket, instance_id, device_id, class, bytes, rejected, shed) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                     ON CONFLICT(bucket, instance_id, device_id, class) DO UPDATE SET \
                       bytes = bytes + excluded.bytes, \
                       rejected = rejected + excluded.rejected, \
                       shed = shed + excluded.shed",
                )
                .expect("语句固定");
            for row in rows {
                stmt.execute(params![
                    row.bucket,
                    row.instance_id,
                    row.device_id,
                    row.class,
                    row.bytes as i64,
                    row.rejected as i64,
                    row.shed as i64,
                ])
                .map_err(|e| e.to_string())?;
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(rows.len())
    }

    /// 某设备 24h 的小时桶字节数（老→新，(bucket, bytes) 升序）。
    pub fn device_traffic_24h(&self, device_id: &str, since_bucket: i64) -> Vec<(i64, i64)> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare(
                "SELECT bucket, SUM(bytes) FROM metrics_hourly \
                 WHERE device_id = ?1 AND bucket >= ?2 GROUP BY bucket ORDER BY bucket",
            )
            .expect("列清单固定");
        let rows = stmt
            .query_map(params![device_id, since_bucket], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })
            .expect("列清单固定");
        rows.filter_map(Result::ok).collect()
    }

    /// 全站最近 N 个小时桶的（交互, 页面/批量）字节，按桶升序。
    pub fn traffic_buckets(&self, since_bucket: i64) -> Vec<(i64, i64, i64)> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare(
                "SELECT bucket, \
                   SUM(CASE WHEN class = 'interactive' THEN bytes ELSE 0 END), \
                   SUM(CASE WHEN class != 'interactive' THEN bytes ELSE 0 END) \
                 FROM metrics_hourly WHERE bucket >= ?1 GROUP BY bucket ORDER BY bucket",
            )
            .expect("列清单固定");
        let rows = stmt
            .query_map(params![since_bucket], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .expect("列清单固定");
        rows.filter_map(Result::ok).collect()
    }

    /// 24h 限流次数：(总计, 其中交互)。
    pub fn throttle_24h(&self, since_bucket: i64) -> (i64, i64) {
        let conn = self.lock();
        conn.query_row(
            "SELECT SUM(rejected + shed), \
             SUM(CASE WHEN class = 'interactive' THEN rejected + shed ELSE 0 END) \
             FROM metrics_hourly WHERE bucket >= ?1",
            params![since_bucket],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .map(|(total, interactive)| (total.max(0), interactive.max(0)))
        .unwrap_or((0, 0))
    }

    /// 某实例最近 1 小时的字节数（概览「流量 · 1h」列）。
    pub fn instance_bytes_1h(&self, instance_id: &str, since_bucket: i64) -> i64 {
        let conn = self.lock();
        conn.query_row(
            "SELECT SUM(bytes) FROM metrics_hourly WHERE instance_id = ?1 AND bucket >= ?2",
            params![instance_id, since_bucket],
            |row| row.get::<_, Option<i64>>(0),
        )
        .ok()
        .flatten()
        .unwrap_or(0)
    }

    /// 每台设备的 24h 合计（设备页「占配额」的分母与分子）。
    pub fn device_bytes_24h(&self, since_bucket: i64) -> std::collections::HashMap<String, i64> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare(
                "SELECT device_id, SUM(bytes) FROM metrics_hourly \
                 WHERE bucket >= ?1 GROUP BY device_id",
            )
            .expect("列清单固定");
        let rows = stmt
            .query_map(params![since_bucket], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .expect("列清单固定");
        rows.filter_map(Result::ok).collect()
    }

    // ---- 配对码 ----

    /// 按明文码查状态。已用的码回填 `device_id`，所以这里能给出设备名——
    /// 状态机的终态要让操作者看见「配对成了哪台设备」。
    pub fn pair_state(&self, code: &str) -> PairState {
        let hash = crypto::sha256_hex(code.as_bytes());
        let conn = self.lock();
        let row: Option<(Option<i64>, i64, Option<String>)> = conn
            .query_row(
                "SELECT p.used_at, p.expires_at, p.device_id FROM pair_codes p \
                 WHERE p.code_hash = ?1",
                params![hash],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .expect("列清单固定");
        let Some((used_at, expires_at, device_id)) = row else {
            return PairState::Unknown;
        };
        if used_at.is_some() {
            let name = device_id.and_then(|id| {
                conn.query_row(
                    "SELECT name FROM devices WHERE id = ?1",
                    params![id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .ok()
                .flatten()
            });
            return PairState::Used { device_name: name };
        }
        if expires_at < now() {
            return PairState::Expired;
        }
        PairState::Pending { expires_at }
    }

    /// 兑换成功后把新建的设备 id 回填到码上（状态机终态要能显示设备名）。
    pub fn bind_pair_device(&self, code: &str, device_id: &str) {
        let hash = crypto::sha256_hex(code.as_bytes());
        let _ = self.lock().execute(
            "UPDATE pair_codes SET device_id = ?2 WHERE code_hash = ?1",
            params![hash, device_id],
        );
    }

    /// 上架一个一次性配对码；返回明文码（5 分钟时效）。
    /// 6 字节 = 48 bit（控制台/千寻发码后手机扫码为主路径，手输也可接受；
    /// 配合 submit 限流 5 次/15min，在线爆破不可行）。
    pub fn create_pair_code(&self, issued_via: &str) -> Result<(String, i64), String> {
        let code = crypto::random_hex(6).to_uppercase();
        let hash = crypto::sha256_hex(code.as_bytes());
        let expires = now() + 300;
        self.lock()
            .execute(
                "INSERT INTO pair_codes (code_hash, issued_via, expires_at) VALUES (?1, ?2, ?3)",
                params![hash, issued_via, expires],
            )
            .map_err(|e| e.to_string())?;
        self.audit(
            "pair.create",
            None,
            None,
            &format!("{{\"via\":\"{issued_via}\"}}"),
        );
        Ok((code, expires))
    }

    /// 原子兑换：未用且未过期才成功，并立刻标记已用（一次性语义）。
    pub fn redeem_pair_code(&self, code: &str) -> Result<&'static str, &'static str> {
        let hash = crypto::sha256_hex(code.as_bytes());
        let conn = self.lock();
        let row: Option<(Option<i64>, i64)> = conn
            .query_row(
                "SELECT used_at, expires_at FROM pair_codes WHERE code_hash = ?1",
                params![hash],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .expect("列清单固定");
        let Some((used_at, expires_at)) = row else {
            return Err("unknown");
        };
        if used_at.is_some() {
            return Err("used");
        }
        if expires_at < now() {
            return Err("expired");
        }
        // 标记已用必须仍是「未用」——并发复兑只有一方成功。
        let used = conn
            .execute(
                "UPDATE pair_codes SET used_at = ?2 WHERE code_hash = ?1 AND used_at IS NULL",
                params![hash, now()],
            )
            .expect("列清单固定");
        if used == 1 {
            Ok("ok")
        } else {
            Err("used")
        }
    }

    // ---- 审计 ----

    pub fn audit(
        &self,
        event: &str,
        instance_id: Option<&str>,
        device_id: Option<&str>,
        detail_json: &str,
    ) {
        let _ = self.lock().execute(
            "INSERT INTO audit (at, instance_id, device_id, event, detail_json) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![now(), instance_id, device_id, event, detail_json],
        );
    }

    /// 审计查询：`kind` 按事件前缀过滤（login/pair/instance/device），
    /// `before_id` 是游标（比偏移量稳：翻页期间有新事件插入也不会错位或跳行）。
    pub fn audit_query(
        &self,
        kind: Option<&str>,
        since: Option<i64>,
        before_id: Option<i64>,
        limit: usize,
    ) -> Vec<AuditRow> {
        let conn = self.lock();
        let kind = kind.filter(|value| !value.is_empty());
        let sql = "SELECT id, at, instance_id, device_id, event, detail_json FROM audit \
                   WHERE (?1 IS NULL OR event LIKE ?1 || '%') \
                     AND (?2 IS NULL OR at >= ?2) \
                     AND (?3 IS NULL OR id < ?3) \
                   ORDER BY id DESC LIMIT ?4";
        let pattern = kind.map(|value| format!("{value}%"));
        let mut stmt = conn.prepare(sql).expect("语句固定");
        let rows = stmt
            .query_map(params![pattern, since, before_id, limit as i64], |row| {
                Ok(AuditRow {
                    id: row.get(0)?,
                    at: row.get(1)?,
                    instance_id: row.get(2)?,
                    device_id: row.get(3)?,
                    event: row.get(4)?,
                    detail_json: row.get(5)?,
                })
            })
            .expect("列清单固定");
        rows.filter_map(Result::ok).collect()
    }

    // ---- 管理面用户与会话（/console）----

    /// 预置用户（fail-closed：无默认账号）。密码 argon2id 哈希后入库；
    /// 明文密码只在返回值里出现一次。
    pub fn add_user(&self, name: &str, password: &str) -> Result<String, String> {
        let id = format!("user-{}", crypto::random_hex(4));
        let hash = hash_password(password)?;
        self.lock()
            .execute(
                "INSERT INTO users (id, name, password_hash, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![id, name, hash, now()],
            )
            .map_err(|e| e.to_string())?;
        Ok(id)
    }

    /// 用户清单（无哈希）。
    pub fn users(&self) -> Vec<(String, String, i64)> {
        let conn = self.lock();
        let mut stmt =
            match conn.prepare("SELECT id, name, created_at FROM users ORDER BY created_at") {
                Ok(stmt) => stmt,
                Err(_) => return Vec::new(),
            };
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }

    /// 改密：吊销该用户全部会话（旧会话立刻失效）。
    pub fn reset_password(&self, name: &str, password: &str) -> Result<(), String> {
        let hash = hash_password(password)?;
        let conn = self.lock();
        let updated = conn
            .execute(
                "UPDATE users SET password_hash = ?2 WHERE name = ?1",
                params![name, hash],
            )
            .map_err(|e| e.to_string())?;
        if updated == 0 {
            return Err("unknown_user".into());
        }
        conn.execute(
            "DELETE FROM sessions WHERE user_id IN (SELECT id FROM users WHERE name = ?1)",
            params![name],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 改名。行 id 不变，所以**已有会话不失效**——改的是登录名，不是身份。
    /// 用改名而不是「删了重建」：删了重建会换一个 user id，
    /// 登录记录（audit 里带 user 的那些）也就对不上人了。
    pub fn rename_user(&self, old: &str, new: &str) -> Result<(), String> {
        let conn = self.lock();
        let updated = conn
            .execute(
                "UPDATE users SET name = ?2 WHERE name = ?1",
                params![old, new],
            )
            .map_err(|e| {
                // users.name 上有 UNIQUE，重名要走人话而不是 SQLITE_CONSTRAINT。
                if e.to_string().contains("UNIQUE") {
                    format!("用户 {new} 已存在")
                } else {
                    e.to_string()
                }
            })?;
        if updated == 0 {
            return Err("unknown_user".into());
        }
        Ok(())
    }

    /// 校验用户名+密码（argon2 verify；用户不存在也做一次哑校验防时序探测）。
    pub fn verify_login(&self, name: &str, password: &str) -> Option<String> {
        let row: Option<(String, String)> = {
            let conn = self.lock();
            conn.query_row(
                "SELECT id, password_hash FROM users WHERE name = ?1",
                params![name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .expect("列清单固定")
        };
        match row {
            Some((id, hash)) if verify_password(password, &hash) => Some(id),
            _ => {
                // 哑校验：用户不存在时消耗等量时间，不泄露账号存在性。
                let _ = verify_password(password, &dummy_hash());
                None
            }
        }
    }

    /// 建会话：返回明文 token（只此一次）；库中只存 SHA-256。
    /// 滑动过期 7 天 / 绝对上限 30 天。
    pub fn create_session(&self, user_id: &str, ip: &str) -> String {
        let token = crypto::random_hex(32);
        let hash = crypto::sha256_hex(token.as_bytes());
        let created = now();
        self.lock()
            .execute(
                "INSERT INTO sessions (token_hash, user_id, created_at, expires_at, last_seen_at, last_ip)
                 VALUES (?1, ?2, ?3, ?4, ?3, ?5)",
                params![hash, user_id, created, created + SESSION_ABSOLUTE_SECS, ip],
            )
            .expect("列清单固定");
        token
    }

    /// 校验会话：有效则滑动续期，返回 user_id；否则 None。
    pub fn validate_session(&self, token: &str) -> Option<String> {
        let hash = crypto::sha256_hex(token.as_bytes());
        let conn = self.lock();
        let row: Option<(String, i64, i64)> = conn
            .query_row(
                "SELECT user_id, expires_at, last_seen_at FROM sessions WHERE token_hash = ?1",
                params![hash],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .expect("列清单固定");
        let (user_id, expires_at, last_seen_at) = row?;
        let current = now();
        if current >= expires_at {
            let _ = conn.execute("DELETE FROM sessions WHERE token_hash = ?1", params![hash]);
            return None;
        }
        // 滑动续期：距上次活跃超过半小时才写库（省写放大）。
        if current - last_seen_at > 1800 {
            let refreshed = (current + SESSION_SLIDING_SECS).min(expires_at);
            let _ = conn.execute(
                "UPDATE sessions SET last_seen_at = ?2, expires_at = ?3 WHERE token_hash = ?1",
                params![hash, current, refreshed],
            );
        }
        Some(user_id)
    }

    /// 登出（吊销单会话）。
    pub fn delete_session(&self, token: &str) {
        let hash = crypto::sha256_hex(token.as_bytes());
        let _ = self
            .lock()
            .execute("DELETE FROM sessions WHERE token_hash = ?1", params![hash]);
    }

    /// 活跃会话数（监控用）。
    pub fn session_count(&self) -> i64 {
        self.lock()
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .unwrap_or(0)
    }

    /// 测试钩子：把会话直接改成已过期。
    #[cfg(test)]
    pub(crate) fn expire_session_for_test(&self, token: &str) {
        let hash = crypto::sha256_hex(token.as_bytes());
        let _ = self.lock().execute(
            "UPDATE sessions SET expires_at = 1 WHERE token_hash = ?1",
            params![hash],
        );
    }
}

/// 会话滑动窗口与绝对上限（秒）。
pub const SESSION_SLIDING_SECS: i64 = 7 * 24 * 3600;
pub const SESSION_ABSOLUTE_SECS: i64 = 30 * 24 * 3600;

/// 哑哈希（用户不存在时的等时校验靶）。
fn dummy_hash() -> String {
    use std::sync::OnceLock;
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY
        .get_or_init(|| hash_password("timing-equalizer").expect("哑哈希生成"))
        .clone()
}

fn hash_password(password: &str) -> Result<String, String> {
    use argon2::password_hash::{PasswordHasher, SaltString};
    let salt = SaltString::generate(&mut rand::rngs::OsRng);
    argon2::Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| e.to_string())
}

fn verify_password(password: &str, hash: &str) -> bool {
    use argon2::PasswordVerifier;
    let Ok(parsed) = argon2::PasswordHash::new(hash) else {
        return false;
    };
    argon2::Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Db {
        Db::open_in_memory().expect("内存库")
    }

    #[test]
    fn 迁移幂等且版本推进() {
        let db = db();
        let version: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);
    }

    #[test]
    fn 用户与会话往返() {
        let db = db();
        db.add_user("admin", "correct-horse").expect("预置用户");
        assert_eq!(db.users().len(), 1);
        // 正确密码 → Some(user_id)；错误/未知用户 → None。
        assert!(db.verify_login("admin", "correct-horse").is_some());
        assert!(db.verify_login("admin", "wrong").is_none());
        assert!(db.verify_login("ghost", "whatever").is_none());
        // 会话：建→验→登出→验失效。
        let user_id = db.verify_login("admin", "correct-horse").unwrap();
        let session = db.create_session(&user_id, "127.0.0.1");
        assert_eq!(db.validate_session(&session), Some(user_id.clone()));
        db.delete_session(&session);
        assert_eq!(db.validate_session(&session), None);
        // 改密 → 全部旧会话吊销。
        let session2 = db.create_session(&user_id, "127.0.0.1");
        db.reset_password("admin", "new-password").expect("改密");
        assert_eq!(db.validate_session(&session2), None);
        assert!(db.verify_login("admin", "new-password").is_some());
    }

    #[test]
    fn 改名换的是登录名不是身份() {
        let db = db();
        let id = db.add_user("manager", "old-password").expect("预置");
        let user_id = db.verify_login("manager", "old-password").expect("登录");
        assert_eq!(id, user_id);
        let session = db.create_session(&user_id, "127.0.0.1");

        db.rename_user("manager", "maxu").expect("改名");

        // 登录名换了，密码没换：旧名字登不进去，新名字进得去。
        assert!(db.verify_login("manager", "old-password").is_none());
        assert!(db.verify_login("maxu", "old-password").is_some());
        // 行 id 没动，所以已登录的会话继续有效——这是「改名」与「删了重建」的关键差别。
        assert_eq!(db.validate_session(&session), Some(user_id));
        assert_eq!(db.users().len(), 1);
        assert_eq!(db.users()[0].0, id, "行 id 不该变");
    }

    #[test]
    fn 改名的两种失败都说人话() {
        let db = db();
        db.add_user("manager", "p").expect("预置");
        db.add_user("maxu", "p").expect("预置");

        // 重名：撞 users.name 的 UNIQUE，但对外不能甩一句 SQLITE_CONSTRAINT。
        let clash = db.rename_user("manager", "maxu").expect_err("重名应失败");
        assert!(clash.contains("已存在"), "实际是：{clash}");
        // 源用户不存在。
        assert_eq!(
            db.rename_user("ghost", "who")
                .expect_err("应为 unknown_user"),
            "unknown_user"
        );
        // 两次失败都没把行弄乱。
        let names: Vec<String> = db.users().into_iter().map(|(_, name, _)| name).collect();
        assert_eq!(names, vec!["manager".to_string(), "maxu".to_string()]);
    }

    #[test]
    fn 会话过期即失效() {
        let db = db();
        db.add_user("u", "p").expect("预置");
        let user_id = db.verify_login("u", "p").unwrap();
        let session = db.create_session(&user_id, "127.0.0.1");
        db.expire_session_for_test(&session);
        assert_eq!(db.validate_session(&session), None);
    }

    #[test]
    fn 实例凭据往返与轮换() {
        let db = db();
        let token = db.add_instance("inst-1", "书房主机").expect("建档");
        assert!(db.instance_auth("inst-1", &token));
        assert!(!db.instance_auth("inst-1", "错的"));
        assert!(!db.instance_auth("inst-2", &token));
        let new_token = db.rotate_instance_token("inst-1").expect("轮换");
        assert!(!db.instance_auth("inst-1", &token), "旧 token 立即失效");
        assert!(db.instance_auth("inst-1", &new_token));
    }

    #[test]
    fn 设备鉴权与撤销() {
        let db = db();
        let token = db.import_device("手机", None).expect("导入");
        let device = db.device_auth(&token).expect("有效");
        assert_eq!(device.name, "手机");
        db.revoke_device(&device.id, "test").expect("撤销");
        assert!(db.device_auth(&token).is_none(), "撤销即时生效");
    }

    /// 撤销必须可逆——这是控制台 5 秒撤销 toast 的前提（docs/20 §4.4）。
    /// 曾经的实现是在撤销路径里轮换 token，那会让「撤销」在物理上不成立。
    #[test]
    fn 撤销只翻标志_恢复后原凭据重新生效() {
        let db = db();
        let token = db.import_device("手机", None).expect("导入");
        let device = db.device_auth(&token).expect("有效");
        db.revoke_device(&device.id, "test").expect("撤销");
        assert!(db.device_auth(&token).is_none());
        db.restore_device(&device.id).expect("恢复");
        let back = db.device_auth(&token).expect("原凭据直接可用");
        assert_eq!(back.token_hash, device.token_hash, "凭据没被动过");
    }

    #[test]
    fn 批量吊销与凭据轮换() {
        let db = db();
        let first = db.import_device("甲", None).expect("导入");
        let second = db.import_device("乙", None).expect("导入");
        assert_eq!(db.revoke_all_devices().expect("批量"), 2);
        assert!(db.device_auth(&first).is_none() && db.device_auth(&second).is_none());
        // 幂等：再点一次没有可吊销的
        assert_eq!(db.revoke_all_devices().expect("再吊销"), 0);
        // 轮换后旧凭据失效、新凭据可用；库里仍只存哈希。
        let id = db.device_rows()[0].id.clone();
        db.restore_device(&id).expect("恢复");
        let (plain, issued, expires) = db.rotate_device_token(&id).expect("轮换");
        assert!(expires > issued);
        assert!(db.device_auth(&plain).is_some());
        let stored = db.device_rows()[0].credential_hint.clone();
        assert!(stored.is_some(), "轮换后有掩码提示");
        assert!(!stored.unwrap().contains(&plain[6..]), "提示不含明文");
    }

    #[test]
    fn 计量按小时桶累加() {
        let db = db();
        let bucket = unix_now().div_euclid(3600) * 3600;
        db.flush_metrics(&[
            MetricRow {
                bucket,
                instance_id: "i1".into(),
                device_id: "dev-a".into(),
                class: "page".into(),
                bytes: 1000,
                rejected: 0,
                shed: 0,
            },
            MetricRow {
                bucket,
                instance_id: "i1".into(),
                device_id: "dev-a".into(),
                class: "page".into(),
                bytes: 500,
                rejected: 2,
                shed: 1,
            },
        ])
        .expect("落库");
        // 同一格累加，不是覆盖。
        assert_eq!(db.device_bytes_24h(bucket - 3600).get("dev-a"), Some(&1500));
        assert_eq!(db.instance_bytes_1h("i1", bucket), 1500);
        assert_eq!(db.throttle_24h(bucket - 3600), (3, 0));
        // 交互类的限流单独计数（概览的两条限流口径）。
        db.flush_metrics(&[MetricRow {
            bucket,
            instance_id: "i1".into(),
            device_id: "dev-a".into(),
            class: "interactive".into(),
            bytes: 10,
            rejected: 4,
            shed: 1,
        }])
        .expect("落库");
        assert_eq!(db.throttle_24h(bucket - 3600), (8, 5));
        // 桶形：只回有流量的桶，缺桶由 web 层的 dense_24 补 0（不跳格）。
        db.flush_metrics(&[MetricRow {
            bucket: bucket - 3600,
            instance_id: "i1".into(),
            device_id: "dev-a".into(),
            class: "page".into(),
            bytes: 7,
            rejected: 0,
            shed: 0,
        }])
        .expect("落库");
        let shape = db.device_traffic_24h("dev-a", bucket - 2 * 3600);
        assert_eq!(shape.len(), 2, "两格有流量");
        assert_eq!(shape[0], (bucket - 3600, 7));
    }

    #[test]
    fn 审计按前缀过滤与游标翻页() {
        let db = db();
        for event in ["login.ok", "login.fail", "pair.grant", "device.revoke"] {
            db.audit(event, None, None, "{}");
        }
        let all = db.audit_query(None, None, None, 10);
        assert_eq!(all.len(), 4, "默认按 id 倒序");
        let logins = db.audit_query(Some("login"), None, None, 10);
        assert_eq!(logins.len(), 2);
        assert!(logins.iter().all(|row| row.event.starts_with("login")));
        // 游标翻页：拿最后一条的 id 当游标，后面不该再有它自己。
        let first_page = db.audit_query(None, None, None, 3);
        let cursor = first_page.last().expect("至少 3 条").id;
        let second_page = db.audit_query(None, None, Some(cursor), 3);
        assert_eq!(second_page.len(), 1);
        assert!(second_page.iter().all(|row| row.id < cursor));
    }

    #[test]
    fn 配对码状态机四态() {
        let db = db();
        let (code, expires) = db.create_pair_code("console").expect("上架");
        assert_eq!(
            db.pair_state(&code),
            PairState::Pending {
                expires_at: expires
            }
        );
        assert_eq!(db.pair_state("ZZZZZZ"), PairState::Unknown);
        db.redeem_pair_code(&code).expect("兑换");
        assert_eq!(db.pair_state(&code), PairState::Used { device_name: None });
    }

    #[test]
    fn 配对码一次性与时效() {
        let db = db();
        let (code, expires) = db.create_pair_code("inst-1").expect("上架");
        assert!(expires > now());
        assert_eq!(db.redeem_pair_code(&code), Ok("ok"));
        assert_eq!(db.redeem_pair_code(&code), Err("used"), "一次性");
        let (other, _) = db.create_pair_code("inst-1").unwrap();
        assert_ne!(code, other);
        assert_eq!(db.redeem_pair_code("FFFFFFFF"), Err("unknown"));
    }

    /// 一个动作只留一条审计。
    ///
    /// 曾经控制台发码/吊销各写两条（`pair.create` + `pair.console_create`），
    /// 审计列表里一次点击读起来像出了两次事。来源改记在 detail 里，事件名只留
    /// 一个——这条测试就是为了不让双写再回来。
    #[test]
    fn 一次动作只写一条审计() {
        let db = db();
        db.create_pair_code("console").expect("上架");
        let code_events = db.audit_query(Some("pair"), None, None, 50);
        assert_eq!(code_events.len(), 1, "发码只有一条：{code_events:?}");
        assert_eq!(code_events[0].event, "pair.create");

        let token = db.import_device("手机", None).expect("导入");
        let device = db.device_auth(&token).expect("有效");
        db.revoke_device(&device.id, "console").expect("吊销");
        let revoke_events = db.audit_query(Some("device.revoke"), None, None, 50);
        assert_eq!(revoke_events.len(), 1, "吊销只有一条：{revoke_events:?}");
    }

    #[test]
    fn 审计带上来源() {
        let db = db();
        db.create_pair_code("console").expect("上架");
        let token = db.import_device("手机", None).expect("导入");
        let device = db.device_auth(&token).expect("有效");
        db.revoke_device(&device.id, "console").expect("吊销");
        let rows = db.audit_query(None, None, None, 10);
        let created = rows.iter().find(|row| row.event == "pair.create").unwrap();
        assert!(
            created.detail_json.contains("\"via\":\"console\""),
            "{}",
            created.detail_json
        );
        let revoked = rows
            .iter()
            .find(|row| row.event == "device.revoke")
            .unwrap();
        assert!(revoked.detail_json.contains("\"via\":\"console\""));
    }

    #[test]
    fn 手工导入保留指定token() {
        let db = db();
        let token = db
            .import_device("旧手机", Some("legacy-token"))
            .expect("导入");
        assert_eq!(token, "legacy-token");
        assert!(db.device_auth("legacy-token").is_some());
    }
}
