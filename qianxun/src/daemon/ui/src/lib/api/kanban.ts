// Kanban API 客户端 (2026-06-04 阶段 3, MVP-3 阶段落地)
// 12 端点封装: 跟 docs/30_子项目规划/_shared-contract.md §8 对齐

import { apiGet, apiPost } from './client';
import type { Board, DispatchedRun, KanbanEvent, Profile, Role, Task } from '$lib/types/kanban';

// ─── Boards (3 端点) ───

export interface BoardsResponse {
  boards: Board[];
}
export interface BoardResponse {
  board: Board;
}

export async function listBoards(): Promise<Board[]> {
  const r = await apiGet<BoardsResponse>('/v1/kanban/boards');
  return r.boards ?? [];
}

export async function getBoard(id: string): Promise<Board> {
  const r = await apiGet<BoardResponse>(`/v1/kanban/boards/${encodeURIComponent(id)}`);
  return r.board;
}

export async function createBoard(name: string, projectRoot: string): Promise<Board> {
  const r = await apiPost<BoardResponse>('/v1/kanban/boards', {
    name,
    project_root: projectRoot
  });
  return r.board;
}

// ─── Tasks (5 端点) ───

export interface TasksResponse {
  tasks: Task[];
  total: number;
  by_status: Record<string, number>;
}
export interface TaskResponse {
  task: Task;
}

export async function listBoardTasks(boardId: string): Promise<Task[]> {
  const r = await apiGet<TasksResponse>(`/v1/kanban/boards/${encodeURIComponent(boardId)}/tasks`);
  return r.tasks ?? [];
}

export async function getTask(id: string): Promise<Task> {
  const r = await apiGet<TaskResponse>(`/v1/kanban/tasks/${encodeURIComponent(id)}`);
  return r.task;
}

export async function createTask(
  boardId: string,
  title: string,
  body: string,
  assigneeRole: string
): Promise<Task> {
  const r = await apiPost<TaskResponse>('/v1/kanban/tasks', {
    board_id: boardId,
    title,
    body,
    assignee_role: assigneeRole
  });
  return r.task;
}

export async function cancelTask(id: string): Promise<void> {
  await apiPost(`/v1/kanban/tasks/${encodeURIComponent(id)}/cancel`);
}

// ─── Events (1 端点) ───

export interface EventsResponse {
  events: KanbanEvent[];
  total: number;
}

export async function listBoardEvents(boardId: string): Promise<KanbanEvent[]> {
  const r = await apiGet<EventsResponse>(
    `/v1/kanban/boards/${encodeURIComponent(boardId)}/events`
  );
  return r.events ?? [];
}

// ─── Dispatch (1 端点) ───

export interface DispatchResponse {
  dispatched: boolean;
  task_id?: string;
  run_id?: string;
  profile_name?: string;
  reason?: string;
}

export async function dispatchNow(prompt: string): Promise<DispatchResponse> {
  return apiPost<DispatchResponse>('/v1/kanban/dispatch', { prompt });
}

// ─── Profiles & Roles (2 端点) ───

export interface ProfilesResponse {
  profiles: Profile[];
  total: number;
}
export interface RolesResponse {
  roles: Role[];
  total: number;
}

export async function listProfiles(): Promise<Profile[]> {
  const r = await apiGet<ProfilesResponse>('/v1/kanban/profiles');
  return r.profiles ?? [];
}

export async function listRoles(): Promise<Role[]> {
  const r = await apiGet<RolesResponse>('/v1/kanban/roles');
  return r.roles ?? [];
}
