//! Kanban 状态机 (v6 §7.1)
//!
//! 7 状态 + 转换规则 + Hermes 关键不变量 `recompute_parent`.
//! 纯函数 (无 IO) + 集成到 `KanbanDb::update_task_status`.

use rusqlite::{params, Connection};

use super::error::KanbanError;
use super::types::TaskStatus;

/// 检查 task 状态转换是否合法 (v6 §7.1).
///
/// 合法转换 (8 种):
/// 1. Triage -> Ready
/// 2. Triage -> Cancelled
/// 3. Triage -> Blocked
/// 4. Ready -> InProgress
/// 5. Ready -> Blocked
/// 6. Ready -> Cancelled
/// 7. InProgress -> Done / Failed / Blocked / Cancelled
/// 8. Blocked -> Ready / Cancelled
/// 9. Failed -> Ready (重试)
pub fn check_transition(from: TaskStatus, to: TaskStatus) -> Result<(), KanbanError> {
    use TaskStatus::*;
    let allowed = match (from, to) {
        // Triage: 分派 (Ready) / 终止 (Cancelled) / 暂缓 (Blocked)
        (Triage, Ready) | (Triage, Cancelled) | (Triage, Blocked) => true,
        // Ready: 派工 (InProgress) / 暂缓 (Blocked) / 终止 (Cancelled)
        (Ready, InProgress) | (Ready, Blocked) | (Ready, Cancelled) => true,
        // InProgress: 完成 (Done) / 失败 (Failed) / 暂缓 (Blocked) / 终止 (Cancelled)
        (InProgress, Done) | (InProgress, Failed) | (InProgress, Blocked) | (InProgress, Cancelled) => true,
        // Blocked: 解除 (Ready) / 终止 (Cancelled)
        (Blocked, Ready) | (Blocked, Cancelled) => true,
        // Done: 终态, 不允许任何转换 (除非重置)
        // Failed: 重试 (Ready)
        (Failed, Ready) => true,
        // 其余: 非法
        _ => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(KanbanError::InvalidStateTransition(
            format!("{from:?}"),
            format!("{to:?}"),
        ))
    }
}

/// 重算父任务状态 (Hermes recompute_ready 的千寻版, v6 §7.1 关键不变量).
///
/// 规则: child 状态变化时, 重新计算 parent 的可执行性.
/// - 全部 children done -> 父从 in_progress -> ready (等待 verifier 门控)
/// - 部分 children done -> 父保持 in_progress
/// - 父无 children (root) -> 不报错, 返 false
///
/// 支持两种父子关系:
/// 1. `kanban_tasks.parent_id` 直接指向 (parent 列)
/// 2. `kanban_task_links` 表间接关联 (DAG 边)
///
/// 返回: 是否触发了父状态变化 (true = 至少一个父从 in_progress 变 ready).
pub fn recompute_parent(conn: &Connection, task_id: &str) -> Result<bool, KanbanError> {
    // 1. 找所有可能的 parent (直接 + 间接 via task_links)
    let direct_parent: Option<String> = conn
        .query_row(
            "SELECT parent_id FROM kanban_tasks WHERE id = ?1",
            params![task_id],
            |row| row.get(0),
        )
        .ok()
        .flatten();

    let link_parents: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT parent_id FROM kanban_task_links WHERE child_id = ?1",
        )?;
        stmt.query_map(params![task_id], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect()
    };

    let mut parents: Vec<String> = direct_parent.into_iter().collect();
    parents.extend(link_parents);
    parents.sort();
    parents.dedup();

    if parents.is_empty() {
        return Ok(false);
    }

    // 2. 对每个 parent 检查所有 children, 全 done 则 in_progress -> ready
    let mut any_triggered = false;
    for parent_id in parents {
        let children_status: Vec<String> = {
            let mut stmt = conn.prepare(
                "SELECT status FROM kanban_tasks WHERE parent_id = ?1 \
                 UNION \
                 SELECT t.status FROM kanban_tasks t \
                 INNER JOIN kanban_task_links l ON l.child_id = t.id \
                 WHERE l.parent_id = ?1",
            )?;
            stmt.query_map(params![parent_id], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect()
        };
        let all_done = !children_status.is_empty()
            && children_status.iter().all(|s| s == "done");
        if all_done {
            let updated = conn.execute(
                "UPDATE kanban_tasks SET status = 'ready' WHERE id = ?1 AND status = 'in_progress'",
                params![parent_id],
            )?;
            if updated > 0 {
                any_triggered = true;
            }
        }
    }
    Ok(any_triggered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kanban::db::str_to_task_status;

    #[test]
    fn test_check_transition_all_8_valid_paths() {
        use TaskStatus::*;
        // Triage -> Ready / Cancelled / Blocked (3)
        assert!(check_transition(Triage, Ready).is_ok());
        assert!(check_transition(Triage, Cancelled).is_ok());
        assert!(check_transition(Triage, Blocked).is_ok());
        // Ready -> InProgress / Blocked / Cancelled (3)
        assert!(check_transition(Ready, InProgress).is_ok());
        assert!(check_transition(Ready, Blocked).is_ok());
        assert!(check_transition(Ready, Cancelled).is_ok());
        // InProgress -> Done / Failed / Blocked / Cancelled (4)
        assert!(check_transition(InProgress, Done).is_ok());
        assert!(check_transition(InProgress, Failed).is_ok());
        assert!(check_transition(InProgress, Blocked).is_ok());
        assert!(check_transition(InProgress, Cancelled).is_ok());
        // Blocked -> Ready / Cancelled (2)
        assert!(check_transition(Blocked, Ready).is_ok());
        assert!(check_transition(Blocked, Cancelled).is_ok());
        // Failed -> Ready (1, retry)
        assert!(check_transition(Failed, Ready).is_ok());
    }

    #[test]
    fn test_check_transition_total_count() {
        // 8 种状态, 8 + 3 + 4 + 2 + 1 = 13 (含 Failed->Ready 重试)
        // 实际看 v6 §7.1: 3+3+4+2+1 = 13
        use TaskStatus::*;
        let valid = [
            (Triage, Ready), (Triage, Cancelled), (Triage, Blocked),
            (Ready, InProgress), (Ready, Blocked), (Ready, Cancelled),
            (InProgress, Done), (InProgress, Failed), (InProgress, Blocked), (InProgress, Cancelled),
            (Blocked, Ready), (Blocked, Cancelled),
            (Failed, Ready),
        ];
        assert_eq!(valid.len(), 13);
        for (from, to) in valid {
            assert!(
                check_transition(from, to).is_ok(),
                "expected {from:?} -> {to:?} to be valid"
            );
        }
    }

    #[test]
    fn test_check_transition_invalid_triage_to_done() {
        use TaskStatus::*;
        // Triage 不能直接 Done (必须先 Ready + InProgress)
        assert!(check_transition(Triage, Done).is_err());
        assert!(check_transition(Triage, InProgress).is_err());
        assert!(check_transition(Triage, Failed).is_err());
    }

    #[test]
    fn test_check_transition_done_is_terminal() {
        use TaskStatus::*;
        // Done 是终态, 不允许任何转换
        assert!(check_transition(Done, Ready).is_err());
        assert!(check_transition(Done, InProgress).is_err());
        assert!(check_transition(Done, Cancelled).is_err());
    }

    #[test]
    fn test_check_transition_invalid_error_contains_states() {
        let err = check_transition(TaskStatus::Triage, TaskStatus::Done).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Triage"), "error should contain Triage: {msg}");
        assert!(msg.contains("Done"), "error should contain Done: {msg}");
    }

    #[test]
    fn test_recompute_parent_all_children_done() {
        let conn = Connection::open_in_memory().expect("in_memory");
        conn.execute_batch(
            "CREATE TABLE kanban_tasks (id TEXT PRIMARY KEY, parent_id TEXT, status TEXT NOT NULL DEFAULT 'triage');
             CREATE TABLE kanban_task_links (parent_id TEXT NOT NULL, child_id TEXT NOT NULL, PRIMARY KEY (parent_id, child_id));"
        ).expect("ddl");
        let now = chrono::Utc::now().to_rfc3339();
        // parent in_progress
        conn.execute(
            "INSERT INTO kanban_tasks (id, status) VALUES ('p', 'in_progress')",
            [],
        ).expect("p");
        // 3 children: 2 done + 1 done (全 done)
        for cid in ["c1", "c2", "c3"] {
            conn.execute(
                "INSERT INTO kanban_tasks (id, parent_id, status) VALUES (?1, 'p', 'done')",
                params![cid],
            ).expect(cid);
            conn.execute(
                "INSERT INTO kanban_task_links (parent_id, child_id) VALUES ('p', ?1)",
                params![cid],
            ).expect("link");
        }
        // 触发 recompute_parent (用任意 child id 即可)
        let triggered = recompute_parent(&conn, "c1").expect("recompute");
        assert!(triggered, "parent should transition to ready");
        // 验证父状态
        let parent_status: String = conn
            .query_row("SELECT status FROM kanban_tasks WHERE id = 'p'", [], |r| r.get(0))
            .expect("query");
        assert_eq!(parent_status, "ready");
        let _ = now; // suppress unused
    }

    #[test]
    fn test_recompute_parent_partial_children_done() {
        let conn = Connection::open_in_memory().expect("in_memory");
        conn.execute_batch(
            "CREATE TABLE kanban_tasks (id TEXT PRIMARY KEY, parent_id TEXT, status TEXT NOT NULL DEFAULT 'triage');
             CREATE TABLE kanban_task_links (parent_id TEXT NOT NULL, child_id TEXT NOT NULL, PRIMARY KEY (parent_id, child_id));"
        ).expect("ddl");
        conn.execute("INSERT INTO kanban_tasks (id, status) VALUES ('p', 'in_progress')", []).expect("p");
        // 1 done, 1 in_progress (不全)
        conn.execute("INSERT INTO kanban_tasks (id, parent_id, status) VALUES ('c1', 'p', 'done')", []).expect("c1");
        conn.execute("INSERT INTO kanban_tasks (id, parent_id, status) VALUES ('c2', 'p', 'in_progress')", []).expect("c2");
        let triggered = recompute_parent(&conn, "c1").expect("recompute");
        assert!(!triggered, "parent should stay in_progress (partial)");
        let parent_status: String = conn
            .query_row("SELECT status FROM kanban_tasks WHERE id = 'p'", [], |r| r.get(0))
            .expect("query");
        assert_eq!(parent_status, "in_progress");
    }

    #[test]
    fn test_recompute_parent_no_parent_noop() {
        // root 任务 (无 parent_id, 不在 task_links) 不报错, 返 false
        let conn = Connection::open_in_memory().expect("in_memory");
        conn.execute_batch(
            "CREATE TABLE kanban_tasks (id TEXT PRIMARY KEY, parent_id TEXT, status TEXT NOT NULL DEFAULT 'triage');
             CREATE TABLE kanban_task_links (parent_id TEXT NOT NULL, child_id TEXT NOT NULL, PRIMARY KEY (parent_id, child_id));"
        ).expect("ddl");
        conn.execute("INSERT INTO kanban_tasks (id, status) VALUES ('root', 'in_progress')", []).expect("root");
        let triggered = recompute_parent(&conn, "root").expect("recompute");
        assert!(!triggered, "root task should not trigger parent transition");
    }

    #[test]
    fn test_recompute_parent_only_via_task_links() {
        // 子任务通过 kanban_task_links 关联 (非 parent_id 列), 也要触发
        let conn = Connection::open_in_memory().expect("in_memory");
        conn.execute_batch(
            "CREATE TABLE kanban_tasks (id TEXT PRIMARY KEY, parent_id TEXT, status TEXT NOT NULL DEFAULT 'triage');
             CREATE TABLE kanban_task_links (parent_id TEXT NOT NULL, child_id TEXT NOT NULL, PRIMARY KEY (parent_id, child_id));"
        ).expect("ddl");
        // parent 没 parent_id (root), child 也没 parent_id, 但通过 links 关联
        conn.execute("INSERT INTO kanban_tasks (id, status) VALUES ('p', 'in_progress')", []).expect("p");
        conn.execute("INSERT INTO kanban_tasks (id, status) VALUES ('c1', 'done')", []).expect("c1");
        conn.execute("INSERT INTO kanban_task_links (parent_id, child_id) VALUES ('p', 'c1')", []).expect("link");
        let triggered = recompute_parent(&conn, "c1").expect("recompute");
        assert!(triggered, "should trigger via task_links even without parent_id");
    }

    #[test]
    fn test_str_to_task_status_all_7() {
        use TaskStatus::*;
        assert_eq!(str_to_task_status("triage"), Some(Triage));
        assert_eq!(str_to_task_status("ready"), Some(Ready));
        assert_eq!(str_to_task_status("in_progress"), Some(InProgress));
        assert_eq!(str_to_task_status("done"), Some(Done));
        assert_eq!(str_to_task_status("blocked"), Some(Blocked));
        assert_eq!(str_to_task_status("cancelled"), Some(Cancelled));
        assert_eq!(str_to_task_status("failed"), Some(Failed));
        assert_eq!(str_to_task_status("unknown"), None);
    }
}
