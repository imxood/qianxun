// Kanban SSE 事件 parser (2026-06-04 阶段 3, 跟现有 chat SSE 共用 /v1/events 端点)

import type { KanbanSseEvent } from '$lib/types/kanban';
export type { KanbanSseEvent };

/**
 * 解析单条 SSE 事件 data 字符串 (JSON). 返 null 表示不是 Kanban 事件
 * (chat event / 系统事件 / 解析错误).
 */
export function parseKanbanEvent(json: string): KanbanSseEvent | null {
  try {
    const o = JSON.parse(json) as KanbanSseEvent;
    switch (o.type) {
      case 'kanban_task_assigned':
      case 'kanban_task_progress':
      case 'kanban_task_completed':
      case 'kanban_task_spawned':
      case 'kanban_blackboard_update':
        return o;
      default:
        return null;
    }
  } catch {
    return null;
  }
}
