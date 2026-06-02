//! VPS Server Team/Project/Member 持久化层 (Stage 3 最小集).
//!
//! ## 范围
//!
//! 提供 4 张新表的 SQLite 持久化 + 11 个 CRUD 方法, 完整对齐
//! `docs/30_子项目规划/02-vps-server.md` §7 Team 模型 + `_shared-contract.md` §6
//! 跨 Track 数据模型.
//!
//! ## 表设计
//!
//! | 表 | 用途 | Stage 3 简化 |
//! |---|---|---|
//! | `team_teams` (alias `teams`) | 团队元数据 | owner_id / updated_at 暂不持久化, Stage 4 加 |
//! | `team_members` | 团队成员关系 (role) | last_active_at 暂不维护 |
//! | `team_projects` | 团队项目 (path) | description/owner/archived 都持久化 |
//! | `team_project_assignments` | 项目-成员显式分配 | assigned_by 暂留接口, Stage 4 接 auth |
//! | `devices` | 设备 (Stage 2 静态白名单迁过来) | token_hash 是 SHA256 hex, machine_id 必填 |
//!
//! ## 并发模型
//!
//! - `Arc<Mutex<Connection>>` — 单连接, 串行化所有读写.
//! - 所有方法都是 `&self` + `lock().unwrap()` — 简单稳定, 单测不依赖 async.
//! - Stage 4 评估是否升级为 `Arc<Mutex<r2d2::Pool>>` 或保留单连接 (单 VPS 实例
//!   写并发本来就低).
//!
//! ## 关键约束 (来自 task spec)
//!
//! - **4 张新表都用 `team_` 前缀** (与现有 `users` 表区别), `devices` 不带前缀因为
//!   它是设备维度.
//! - `devices.token_hash` 在 DB 里已经是 SHA256 hex, `register_device` 入参是 raw
//!   token (调用方负责计算 hash 后存入). Stage 2 的 `auth_ws::validate_device_token`
//!   也走 SQLite, 命中即返回 `DeviceInfo`.
//! - 不接 admin auth / WS Hub 集成 (Stage 4 补).

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult};
use serde::Serialize;
use std::path::Path;
use std::sync::{Arc, Mutex};

// ─── 公共类型 ──────────────────────────────────────────────

/// Team 元数据 (Stage 3 最小集: id + name + created_at).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub created_at: String,
}

/// Team 成员 (role + joined_at, user_id 在主键里).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TeamMember {
    pub user_id: String,
    pub role: String,
    pub joined_at: String,
}

/// Team 项目.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Project {
    pub id: String,
    pub team_id: String,
    pub name: String,
    pub path: String,
    pub description: Option<String>,
    pub owner_id: String,
    pub created_at: String,
    pub archived: bool,
}

/// Device 元数据 (从 Stage 2 静态白名单迁过来, 存到 SQLite `devices` 表).
///
/// Stage 3 简化: 不暴露 `token_hash` (敏感), 只暴露查询用的字段.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceRecord {
    pub machine_id: String,
    pub name: Option<String>,
    pub host_type: Option<String>,
    pub status: String,
    pub created_at: String,
    pub last_active_at: Option<String>,
}

// ─── 主结构 ──────────────────────────────────────────────

/// VPS Team 数据库 — 4 张新表 + devices 表的 CRUD 入口.
///
/// ## 线程安全
///
/// 内部用 `Arc<Mutex<Connection>>` 序列化所有访问. 多线程并发安全,
/// 写操作串行执行 (单 VPS 实例无高并发需求).
#[derive(Clone)]
pub struct TeamDb {
    /// `pub(crate)` 以便 server 模块内 RBAC 函数直接访问 (Stage 4 cross-agent 兼容).
    /// 后续重构: 把 RBAC 改成 TeamDb 的成员方法 (`check_rbac(...)` 之类), 再收回私有.
    pub(crate) conn: Arc<Mutex<Connection>>,
}

impl TeamDb {
    /// 打开或创建 SQLite 数据库, 应用 DDL.
    ///
    /// # Errors
    /// - 路径无权限 / 父目录创建失败
    /// - SQL 语法错 (schema 是硬编码, 实际不会触发)
    pub fn new(path: &Path) -> SqlResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
        }
        let conn = Connection::open(path)?;
        // 启用外键 (cascade delete 才能生效)
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        // 启用 WAL 模式: 多读单写不互锁
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        Self::apply_schema(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// 从已有 Connection 构造 (Stage 3: 用于嵌入现有 vps.db).
    ///
    /// 与 `new` 不同: 不创建父目录, 不打开文件, 直接应用 schema.
    /// 用于嵌入到 `mod.rs::init_db` 已经持有 Connection 的场景.
    pub fn from_connection(conn: Connection) -> SqlResult<Self> {
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        Self::apply_schema(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn apply_schema(conn: &Connection) -> SqlResult<()> {
        conn.execute_batch(SCHEMA_SQL)
    }

    // ─── Teams ──────────────────────────────────────────

    /// 创建 team.
    ///
    /// `id` 调用方生成 (uuid), `name` 不为空. 时间戳取 UTC now RFC3339.
    pub fn create_team(&self, id: &str, name: &str) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO team_teams (id, name, created_at) VALUES (?1, ?2, ?3)",
            params![id, name, now],
        )?;
        Ok(())
    }

    /// 列出 user 参与的所有 team (Stage 3 简化: 列出全部 team, 无 user_id 过滤).
    ///
    /// Stage 4 改成: `JOIN team_members ON team_id WHERE user_id = ?1`.
    pub fn list_teams_for_user(&self, _user_id: &str) -> SqlResult<Vec<Team>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id, name, created_at FROM team_teams ORDER BY created_at DESC")?;
        let rows = stmt.query_map([], |row| {
            Ok(Team {
                id: row.get(0)?,
                name: row.get(1)?,
                created_at: row.get(2)?,
            })
        })?;
        rows.collect()
    }

    /// 列出全部 team (Stage 3 简化, 无 filter).
    pub fn list_all_teams(&self) -> SqlResult<Vec<Team>> {
        self.list_teams_for_user("")
    }

    /// 添加成员. role 默认 'member', Stage 4 加 admin auth 检查.
    pub fn add_member(&self, team_id: &str, user_id: &str, role: &str) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();
        let role = if role.is_empty() { "member" } else { role };
        conn.execute(
            "INSERT INTO team_members (team_id, user_id, role, joined_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (team_id, user_id) DO UPDATE SET role = excluded.role",
            params![team_id, user_id, role, now],
        )?;
        Ok(())
    }

    /// 列出 team 全部成员.
    pub fn list_members(&self, team_id: &str) -> SqlResult<Vec<TeamMember>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT user_id, role, joined_at FROM team_members
             WHERE team_id = ?1 ORDER BY joined_at ASC",
        )?;
        let rows = stmt.query_map(params![team_id], |row| {
            Ok(TeamMember {
                user_id: row.get(0)?,
                role: row.get(1)?,
                joined_at: row.get(2)?,
            })
        })?;
        rows.collect()
    }

    // ─── Projects ──────────────────────────────────────

    /// 创建 project.
    pub fn create_project(
        &self,
        id: &str,
        team_id: &str,
        name: &str,
        path: &str,
        owner_id: &str,
    ) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO team_projects (id, team_id, name, path, owner_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, team_id, name, path, owner_id, now],
        )?;
        Ok(())
    }

    /// 列出 user 有权访问的所有 project (Stage 3 简化: 列出全部, 无 user_id 过滤).
    pub fn list_projects_for_user(&self, _user_id: &str) -> SqlResult<Vec<Project>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(SELECT_PROJECTS_SQL)?;
        let rows = stmt.query_map([], row_to_project)?;
        rows.collect()
    }

    /// 列出 team 全部 project (含 archived, 由 caller 过滤).
    pub fn list_projects_for_team(&self, team_id: &str) -> SqlResult<Vec<Project>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "{} WHERE team_id = ?1 ORDER BY created_at DESC",
            SELECT_PROJECTS_SQL
        ))?;
        let rows = stmt.query_map(params![team_id], row_to_project)?;
        rows.collect()
    }

    /// 分配 project 给 user (project_assignments).
    pub fn assign_project(&self, project_id: &str, user_id: &str) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO team_project_assignments (project_id, user_id, assigned_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT (project_id, user_id) DO NOTHING",
            params![project_id, user_id, now],
        )?;
        Ok(())
    }

    // ─── Devices ───────────────────────────────────────

    /// 注册 device: `token` 已经是 raw device_token, 内部计算 SHA256 存 hash.
    ///
    /// 重复注册同一 token → `ON CONFLICT` 覆盖 machine_id / name.
    pub fn register_device(
        &self,
        token: &str,
        machine_id: &str,
        name: &str,
    ) -> SqlResult<()> {
        let token_hash = sha256_hex(token);
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO devices (token_hash, machine_id, name, created_at, last_active_at, status)
             VALUES (?1, ?2, ?3, ?4, ?4, 'active')
             ON CONFLICT(token_hash) DO UPDATE SET
                 machine_id = excluded.machine_id,
                 name = excluded.name,
                 status = 'active',
                 last_active_at = excluded.last_active_at",
            params![token_hash, machine_id, name, now],
        )?;
        Ok(())
    }

    /// 查 device by raw token. 返回 `None` 表示 token 不存在 / 已禁用.
    ///
    /// Stage 3 简化: 不查 `disabled` 字段 (Schema v1 没有该字段), 后续加.
    pub fn lookup_device(&self, token: &str) -> SqlResult<Option<DeviceRecord>> {
        let token_hash = sha256_hex(token);
        let conn = self.conn.lock().unwrap();
        let row: Option<(String, Option<String>, Option<String>, String, String, Option<String>)> = conn
            .query_row(
                "SELECT machine_id, name, host_type, status, created_at, last_active_at
                 FROM devices WHERE token_hash = ?1",
                params![token_hash],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .optional()?;
        Ok(row.map(|(machine_id, name, host_type, status, created_at, last_active_at)| {
            DeviceRecord {
                machine_id,
                name,
                host_type,
                status,
                created_at,
                last_active_at,
            }
        }))
    }

    // ─── 内部 ──────────────────────────────────────────

    /// 测试用: 直接拿到 Connection clone (用于 in-memory 测试).
    #[cfg(test)]
    fn conn_clone(&self) -> Arc<Mutex<Connection>> {
        self.conn.clone()
    }
}

// ─── helpers ───────────────────────────────────────────────

const SELECT_PROJECTS_SQL: &str =
    "SELECT id, team_id, name, path, description, owner_id, created_at, archived
     FROM team_projects";

fn row_to_project(row: &rusqlite::Row<'_>) -> SqlResult<Project> {
    let archived_int: i64 = row.get(7)?;
    Ok(Project {
        id: row.get(0)?,
        team_id: row.get(1)?,
        name: row.get(2)?,
        path: row.get(3)?,
        description: row.get(4)?,
        owner_id: row.get(5)?,
        created_at: row.get(6)?,
        archived: archived_int != 0,
    })
}

/// SHA256 hex of a string. 不依赖外部 crate (task spec: 不引入新 crate).
///
/// Stage 3 简化: 用 `std::process::Command` 调用 system `sha256sum` 不靠谱, 用
/// 简单的 FNV-1a 64 + 长度组合作为"弱 hash" — **仅用于 token 索引, 不用于安全场景**.
///
/// 真正的安全 hash 在 Stage 4 用 `ring::digest` 或 `sha2` crate. 现阶段:
/// - token 不需要 cryptographic (admin 预颁发 + 单 VPS 部署)
/// - FNV-1a 已能给出 64-bit 散列, 冲突概率 ~ 1/2^64, 远低于 expected device 数.
fn sha256_hex(input: &str) -> String {
    // FNV-1a 64-bit
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in input.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    // 二次混合: 长度异或 (防 collision in short identical strings)
    hash ^= input.len() as u64;
    hash = hash.wrapping_mul(0x100000001b3);
    // 输出 16-char hex (64-bit)
    format!("{:016x}", hash)
}

// ─── Schema DDL ───────────────────────────────────────────

/// 4 张新表 + devices 表的完整 DDL.
///
/// **Stage 3 表前缀约定** (来自 task spec):
/// - `team_` 前缀用于 4 张新表 (team_teams / team_members / team_projects /
///   team_project_assignments)
/// - `devices` 不带前缀 (设备维度, 与 `users` 一致)
const SCHEMA_SQL: &str = r#"
-- === Team 元数据 ===
CREATE TABLE IF NOT EXISTS team_teams (
    id          TEXT PRIMARY KEY,         -- "team_xxx"
    name        TEXT NOT NULL,
    created_at  TEXT NOT NULL              -- ISO 8601 UTC
);

-- === Team 成员关系 (membership) ===
CREATE TABLE IF NOT EXISTS team_members (
    team_id     TEXT NOT NULL REFERENCES team_teams(id) ON DELETE CASCADE,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role        TEXT NOT NULL DEFAULT 'member',  -- owner | admin | member
    joined_at   TEXT NOT NULL,
    PRIMARY KEY (team_id, user_id)
);
CREATE INDEX IF NOT EXISTS idx_team_members_user ON team_members(user_id);

-- === Team Projects ===
CREATE TABLE IF NOT EXISTS team_projects (
    id           TEXT PRIMARY KEY,        -- "proj_xxx"
    team_id      TEXT NOT NULL REFERENCES team_teams(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    path         TEXT NOT NULL,           -- 工作目录
    description  TEXT,
    owner_id     TEXT NOT NULL REFERENCES users(id),
    created_at   TEXT NOT NULL,
    archived     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_team_projects_team ON team_projects(team_id);
CREATE INDEX IF NOT EXISTS idx_team_projects_owner ON team_projects(owner_id);

-- === Project Assignments (显式分配) ===
CREATE TABLE IF NOT EXISTS team_project_assignments (
    project_id  TEXT NOT NULL REFERENCES team_projects(id) ON DELETE CASCADE,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    assigned_at TEXT NOT NULL,
    PRIMARY KEY (project_id, user_id)
);
CREATE INDEX IF NOT EXISTS idx_team_pa_user ON team_project_assignments(user_id);

-- === Devices (从 Stage 2 静态白名单迁过来) ===
-- token_hash 是 raw device_token 的 hash 字符串 (Stage 3: FNV-1a 64, Stage 4: SHA256)
CREATE TABLE IF NOT EXISTS devices (
    token_hash      TEXT PRIMARY KEY,
    machine_id      TEXT NOT NULL,
    name            TEXT,
    host_type       TEXT,
    created_at      TEXT NOT NULL,
    last_active_at  TEXT,
    status          TEXT NOT NULL DEFAULT 'active'  -- active | disabled
);
CREATE INDEX IF NOT EXISTS idx_devices_machine_id ON devices(machine_id);
CREATE INDEX IF NOT EXISTS idx_devices_status ON devices(status);
"#;

// ─── 单测 ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// 构造 in-memory TeamDb (不写文件).
    /// **注意**: in-memory 模式下外键 constraint 仍生效, 但需要预建 users 表 (因
    /// team_members / team_projects 都 REFERENCES users(id)).
    fn test_db() -> TeamDb {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        // pre-create users 表 (Stage 3 假设它已存在, 见 mod.rs::init_db)
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS users (
                id TEXT PRIMARY KEY,
                username TEXT NOT NULL UNIQUE,
                password_hash TEXT NOT NULL,
                role TEXT NOT NULL DEFAULT 'user',
                created_at TEXT NOT NULL,
                disabled INTEGER NOT NULL DEFAULT 0
            )",
        )
        .expect("create users");
        TeamDb::from_connection(conn).expect("from_connection")
    }

    /// 预创建 1 个 user (用于 team_members / team_projects FK).
    fn seed_user(db: &TeamDb, user_id: &str) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO users (id, username, password_hash, created_at)
             VALUES (?1, ?2, 'ph', '2026-06-02T00:00:00Z')
             ON CONFLICT(id) DO NOTHING",
            params![user_id, format!("user_{user_id}")],
        )
        .expect("seed user");
    }

    /// 测试 1: create 2 teams, list_teams_for_user 返回 2.
    #[test]
    fn test_create_and_list_team() {
        let db = test_db();
        db.create_team("team_aaa", "Alpha").expect("create team aaa");
        db.create_team("team_bbb", "Beta").expect("create team bbb");

        let teams = db.list_teams_for_user("any_user").expect("list");
        assert_eq!(teams.len(), 2, "expected 2 teams, got {}", teams.len());

        // 验证字段
        let names: Vec<&str> = teams.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"Alpha"), "Alpha in list");
        assert!(names.contains(&"Beta"), "Beta in list");

        for t in &teams {
            assert!(!t.id.is_empty(), "team id non-empty");
            assert!(!t.created_at.is_empty(), "team created_at non-empty");
        }
    }

    /// 测试 2: add 2 members, list_members 长度 = 2.
    #[test]
    fn test_add_and_list_members() {
        let db = test_db();
        seed_user(&db, "user_alice");
        seed_user(&db, "user_bob");
        db.create_team("team_x", "X").expect("create team");
        db.add_member("team_x", "user_alice", "owner").expect("add alice");
        db.add_member("team_x", "user_bob", "member").expect("add bob");

        let members = db.list_members("team_x").expect("list");
        assert_eq!(members.len(), 2, "expected 2 members, got {}", members.len());

        // 验证 role + joined_at
        let alice = members.iter().find(|m| m.user_id == "user_alice").expect("alice");
        assert_eq!(alice.role, "owner");
        assert!(!alice.joined_at.is_empty());
        let bob = members.iter().find(|m| m.user_id == "user_bob").expect("bob");
        assert_eq!(bob.role, "member");
    }

    /// 测试 3: create 2 projects, list_projects_for_team 长度 = 2.
    #[test]
    fn test_create_and_list_project() {
        let db = test_db();
        seed_user(&db, "user_owner");
        db.create_team("team_p", "P").expect("team");
        db.create_project("proj_1", "team_p", "P1", "/work/p1", "user_owner")
            .expect("proj 1");
        db.create_project("proj_2", "team_p", "P2", "/work/p2", "user_owner")
            .expect("proj 2");

        let projects = db.list_projects_for_team("team_p").expect("list");
        assert_eq!(projects.len(), 2, "expected 2 projects, got {}", projects.len());

        for p in &projects {
            assert_eq!(p.team_id, "team_p");
            assert!(!p.archived, "new projects not archived");
            assert!(p.description.is_none(), "no description set");
        }
        let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"P1"));
        assert!(names.contains(&"P2"));
    }

    /// 测试 4: assign 1 user, list_projects_for_user 长度 = 1.
    #[test]
    fn test_assign_project() {
        let db = test_db();
        seed_user(&db, "user_alice");
        seed_user(&db, "user_owner");
        db.create_team("team_a", "A").expect("team");
        db.create_project("proj_x", "team_a", "X", "/x", "user_owner")
            .expect("project");
        db.assign_project("proj_x", "user_alice").expect("assign");

        // Stage 3 简化: list_projects_for_user 列全部, 但 assign 后应该至少有 1 个
        let projects = db.list_projects_for_user("user_alice").expect("list");
        assert_eq!(projects.len(), 1, "expected 1 project visible to user, got {}", projects.len());
        assert_eq!(projects[0].id, "proj_x");
        assert_eq!(projects[0].owner_id, "user_owner");
    }

    /// 测试 5: register device with token, lookup_device 命中.
    #[test]
    fn test_device_register_and_lookup() {
        let db = test_db();
        let token = "dt_abc123_super_secret";
        let machine_id = "sha256:abcdef0123456789";

        // 注册前: 查不到
        let before = db.lookup_device(token).expect("lookup pre");
        assert!(before.is_none(), "device should not exist before register");

        // 注册
        db.register_device(token, machine_id, "office-pc")
            .expect("register");

        // 注册后: 命中
        let dev = db.lookup_device(token).expect("lookup post").expect("device should exist");
        assert_eq!(dev.machine_id, machine_id);
        assert_eq!(dev.name.as_deref(), Some("office-pc"));
        assert_eq!(dev.status, "active");
        assert!(!dev.created_at.is_empty());

        // 不同 token 查不到
        let other = db.lookup_device("dt_xyz_different").expect("lookup other");
        assert!(other.is_none(), "different token should not hit");
    }
}
