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

/// 设备（手机等消费端）的注册记录；token 只存哈希。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceRow {
    pub id: String,
    pub name: String,
    pub token_hash: String,
    pub created_at: i64,
    pub last_seen_at: i64,
    pub revoked: bool,
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
        self.lock()
            .execute(
                "INSERT INTO devices (id, name, token_hash, created_at, last_seen_at) VALUES (?1, ?2, ?3, ?4, ?4)",
                params![id, name, hash, now()],
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
                "SELECT id, name, token_hash, created_at, last_seen_at, revoked FROM devices WHERE token_hash = ?1",
                params![hash],
                |row| {
                    Ok(DeviceRow {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        token_hash: row.get(2)?,
                        created_at: row.get(3)?,
                        last_seen_at: row.get(4)?,
                        revoked: row.get::<_, i64>(5)? != 0,
                    })
                },
            )
            .optional()
            .expect("列清单固定");
        row.filter(|device| !device.revoked)
    }

    pub fn revoke_device(&self, id: &str) -> Result<(), String> {
        let changed = self
            .lock()
            .execute("UPDATE devices SET revoked = 1 WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err(format!("设备 {id} 不存在"));
        }
        self.audit("device.revoke", None, Some(id), "{}");
        Ok(())
    }

    pub fn touch_device(&self, id: &str) {
        let _ = self.lock().execute(
            "UPDATE devices SET last_seen_at = ?2 WHERE id = ?1",
            params![id, now()],
        );
    }

    // ---- 配对码 ----

    /// 上架一个一次性配对码；返回明文码（5 分钟时效）。
    pub fn create_pair_code(&self, issued_via: &str) -> Result<(String, i64), String> {
        let code = crypto::random_hex(4).to_uppercase();
        let hash = crypto::sha256_hex(code.as_bytes());
        let expires = now() + 300;
        self.lock()
            .execute(
                "INSERT INTO pair_codes (code_hash, issued_via, expires_at) VALUES (?1, ?2, ?3)",
                params![hash, issued_via, expires],
            )
            .map_err(|e| e.to_string())?;
        self.audit("pair.create", None, None, "{}");
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
        db.revoke_device(&device.id).expect("撤销");
        assert!(db.device_auth(&token).is_none(), "撤销即时生效");
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
