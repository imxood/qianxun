// 千寻 Kanban 子系统前端类型 (2026-06-04 阶段 3, MVP-2 + MVP-3)
// 跟 qianxun-core/src/kanban/types.rs 字段对齐 (snake_case 跟 daemon 一致)

export type TaskStatus =
  | 'triage'
  | 'ready'
  | 'in_progress'
  | 'done'
  | 'blocked'
  | 'cancelled'
  | 'failed';

export type RunOutcome = 'success' | 'partial_success' | 'failure' | 'skipped' | 'gate_blocked';

export type BoardStatus = 'active' | 'archived';
export type ProjectStatus = 'active' | 'archived';

export type ProfileKind = 'local' | 'remote';
export type WorkerScope = 'Worker' | 'Orchestrator';

export interface Project {
  id: string;
  name: string;
  description: string;
  default_root: string;
  extra_roots: string[];
  status: ProjectStatus;
  owner: string;
  created_at: string;
  updated_at: string;
}

export interface KanbanBoard {
  id: string;
  project_id: string;
  name: string;
  project_root: string;
  default_role: string;
  status: BoardStatus;
  created_at: string;
  updated_at: string;
}

// 别名 (跟 api/kanban.ts 命名一致)
export type Board = KanbanBoard;

export interface Task {
  id: string;
  board_id: string;
  parent_id: string | null;
  title: string;
  body: string;
  assignee_role: string;
  status: TaskStatus;
  priority: number;
  deadline: string | null;
  metadata: Record<string, unknown>;
  created_at: string;
  t_started_at: string | null;
  t_completed_at: string | null;
  last_heartbeat_at: string | null;
}

export interface Run {
  id: string;
  task_id: string;
  profile_id: string;
  status: string;
  claim_id: string;
  r_heartbeat_at: string | null;
  started_at: string;
  ended_at: string | null;
  outcome: RunOutcome;
  summary: string;
  error: string | null;
  token_input: number;
  token_output: number;
}

export interface Profile {
  id: string;
  name: string;
  kind: ProfileKind;
  working_dir: string;
  max_turns: number;
  model: string | null;
  system_prompt_template: string;
}

export interface Role {
  id: string;
  name: string;
  description: string;
  instructions: string;
  default_profile_id: string;
  allowed_tool_categories: string[];
}

export interface KanbanEvent {
  id: number;
  task_id: string | null;
  run_id: string | null;
  kind: string;
  payload: Record<string, unknown>;
  created_at: string;
}

export interface DispatchedRun {
  task_id: string;
  run_id: string;
  profile_name: string;
}

// SSE 事件 (SseEvent 5 个 Kanban variant)
export type KanbanSseEvent =
  | { type: 'kanban_task_assigned'; task_id: string; run_id: string; profile_name: string; title: string }
  | { type: 'kanban_task_progress'; task_id: string; run_id: string; event_kind: string; preview: string }
  | {
      type: 'kanban_task_completed';
      task_id: string;
      run_id: string;
      outcome: string;
      summary: string;
      token_input: number;
      token_output: number;
      elapsed_ms: number;
    }
  | {
      type: 'kanban_task_spawned';
      parent_task_id: string | null;
      child_task_id: string;
      title: string;
      assignee_role: string;
    }
  | { type: 'kanban_blackboard_update'; task_id: string; key: string; value_preview: string };
