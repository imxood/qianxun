//! 3 张 daemon_* 表 DDL + 8 张 kanban_* 表 DDL (从 persistence.rs 抽, 2026-06-04 Commit 11)

use rusqlite::Connection;

use super::kanban_schema::init_kanban_schema;

/// 初始化 3 张 daemon_sessions/snapshots/event_log + 8 张 kanban_* 表.
pub fn create_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "
        -- === 1. session 元数据 + 状态机 ===
        CREATE TABLE IF NOT EXISTS daemon_sessions (
            id              TEXT PRIMARY KEY,
            project_root    TEXT,
            config_json     TEXT NOT NULL,
            status          TEXT NOT NULL DEFAULT 'active',
            created_at      TEXT NOT NULL,
            last_active_at  TEXT NOT NULL,
            message_count   INTEGER NOT NULL DEFAULT 0
        );

        -- === 2. conversation 快照 (增量) ===
        CREATE TABLE IF NOT EXISTS daemon_conversation_snapshots (
            session_id      TEXT NOT NULL REFERENCES daemon_sessions(id) ON DELETE CASCADE,
            ordinal         INTEGER NOT NULL,
            data_json       TEXT NOT NULL,
            created_at      TEXT NOT NULL,
            PRIMARY KEY (session_id, ordinal)
        );

        -- === 3. 事件流 (SSE transcript) ===
        CREATE TABLE IF NOT EXISTS daemon_event_log (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id      TEXT NOT NULL REFERENCES daemon_sessions(id) ON DELETE CASCADE,
            seq             INTEGER NOT NULL,
            event_type      TEXT NOT NULL,
            event_json      TEXT NOT NULL,
            created_at      TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_event_log_session_seq
            ON daemon_event_log(session_id, seq);

        -- === 4-12. Kanban 子系统 (MVP-2 plan 1) — 8 张 kanban_* 表 ===
        -- 见 v6 §6.5 + §3.6.2. 沿用 daemon.db, 跟 daemon_sessions 同一文件.
        -- 字段名 t_/r_ 前缀 (v6 §6.2 决策, 避免 SQL JOIN 混淆).

        -- §3.6.2 Project (v5 新增) — 1:N Board
        CREATE TABLE IF NOT EXISTS kanban_projects (
            id            TEXT PRIMARY KEY,
            name          TEXT NOT NULL,
            description   TEXT NOT NULL,
            default_root  TEXT NOT NULL,
            extra_roots   TEXT NOT NULL DEFAULT '[]',
            status        TEXT NOT NULL DEFAULT 'active',
            owner         TEXT NOT NULL DEFAULT 'local',
            created_at    TEXT NOT NULL,
            updated_at    TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_kanban_projects_status ON kanban_projects(status);

        -- §6.5 Board — 1:1 Project (v1 简化, v2 多 board)
        CREATE TABLE IF NOT EXISTS kanban_boards (
            id            TEXT PRIMARY KEY,
            project_id    TEXT REFERENCES kanban_projects(id) ON DELETE CASCADE,
            name          TEXT NOT NULL,
            project_root  TEXT NOT NULL,
            default_role  TEXT NOT NULL DEFAULT 'coordinator',
            status        TEXT NOT NULL DEFAULT 'active',
            created_at    TEXT NOT NULL,
            updated_at    TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_kanban_boards_status ON kanban_boards(status);
        CREATE INDEX IF NOT EXISTS idx_kanban_boards_project ON kanban_boards(project_id);

        -- §6.5 Role — 角色模板
        CREATE TABLE IF NOT EXISTS kanban_role_defs (
            id            TEXT PRIMARY KEY,
            name          TEXT NOT NULL UNIQUE,
            description   TEXT NOT NULL,
            instructions  TEXT NOT NULL,
            default_profile_id TEXT,
            allowed_tool_categories TEXT NOT NULL,
            created_at    TEXT NOT NULL
        );

        -- §6.5 Profile
        CREATE TABLE IF NOT EXISTS kanban_profiles (
            id            TEXT PRIMARY KEY,
            name          TEXT NOT NULL UNIQUE,
            kind          TEXT NOT NULL DEFAULT 'local',
            working_dir   TEXT NOT NULL,
            tool_filter   TEXT NOT NULL,
            max_turns     INTEGER NOT NULL DEFAULT 32,
            model         TEXT,
            system_prompt_template TEXT NOT NULL,
            created_at    TEXT NOT NULL
        );

        -- §6.5 Task (t_/r_ 前缀)
        CREATE TABLE IF NOT EXISTS kanban_tasks (
            id            TEXT PRIMARY KEY,
            board_id      TEXT NOT NULL REFERENCES kanban_boards(id) ON DELETE CASCADE,
            parent_id     TEXT REFERENCES kanban_tasks(id) ON DELETE CASCADE,
            title         TEXT NOT NULL,
            body          TEXT NOT NULL,
            assignee_role TEXT NOT NULL,
            status        TEXT NOT NULL DEFAULT 'triage',
            priority      INTEGER NOT NULL DEFAULT 128,
            deadline      TEXT,
            metadata      TEXT NOT NULL DEFAULT '{}',
            created_at    TEXT NOT NULL,
            t_started_at  TEXT,
            t_completed_at TEXT,
            last_heartbeat_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_kanban_tasks_board ON kanban_tasks(board_id);
        CREATE INDEX IF NOT EXISTS idx_kanban_tasks_parent ON kanban_tasks(parent_id);
        CREATE INDEX IF NOT EXISTS idx_kanban_tasks_status ON kanban_tasks(status);
        CREATE INDEX IF NOT EXISTS idx_kanban_tasks_assignee ON kanban_tasks(assignee_role);

        -- §6.5 TaskLink — DAG 边
        CREATE TABLE IF NOT EXISTS kanban_task_links (
            parent_id     TEXT NOT NULL REFERENCES kanban_tasks(id) ON DELETE CASCADE,
            child_id      TEXT NOT NULL REFERENCES kanban_tasks(id) ON DELETE CASCADE,
            dep_type      TEXT NOT NULL DEFAULT 'sequential',
            created_at    TEXT NOT NULL,
            PRIMARY KEY (parent_id, child_id)
        );
        CREATE INDEX IF NOT EXISTS idx_kanban_task_links_child ON kanban_task_links(child_id);

        -- §6.5 AgentRun
        CREATE TABLE IF NOT EXISTS kanban_runs (
            id            TEXT PRIMARY KEY,
            task_id       TEXT NOT NULL REFERENCES kanban_tasks(id) ON DELETE CASCADE,
            profile_id    TEXT NOT NULL REFERENCES kanban_profiles(id),
            status        TEXT NOT NULL DEFAULT 'pending',
            claim_id      TEXT NOT NULL,
            r_heartbeat_at TEXT,
            started_at    TEXT NOT NULL,
            ended_at      TEXT,
            outcome       TEXT NOT NULL DEFAULT 'success',
            summary       TEXT NOT NULL DEFAULT '',
            error         TEXT,
            token_input   INTEGER NOT NULL DEFAULT 0,
            token_output  INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_kanban_runs_task ON kanban_runs(task_id);
        CREATE INDEX IF NOT EXISTS idx_kanban_runs_status ON kanban_runs(status);

        -- §4 Blackboard
        CREATE TABLE IF NOT EXISTS kanban_blackboard (
            task_id       TEXT NOT NULL REFERENCES kanban_tasks(id) ON DELETE CASCADE,
            key           TEXT NOT NULL,
            value         TEXT NOT NULL,
            author        TEXT NOT NULL,
            updated_at    TEXT NOT NULL,
            PRIMARY KEY (task_id, key)
        );
        CREATE INDEX IF NOT EXISTS idx_kanban_blackboard_task ON kanban_blackboard(task_id);

        -- §6.3 事件
        CREATE TABLE IF NOT EXISTS kanban_events (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id       TEXT,
            run_id        TEXT,
            kind          TEXT NOT NULL,
            payload       TEXT NOT NULL,
            created_at    TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_kanban_events_task ON kanban_events(task_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_kanban_events_kind ON kanban_events(kind);
        ",
    )?;
    // ALTER 迁移 + default project 注入 + 老数据归位
    init_kanban_schema(conn)?;
    Ok(())
}
