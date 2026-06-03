// Projects API 客户端 (2026-06-04 阶段 3, MVP-3 落地 2 端点)

import { apiGet, apiPost } from './client';
import type { Project } from '$lib/types/kanban';

export interface ProjectsResponse {
  projects: Project[];
}
export interface ProjectResponse {
  project: Project;
}

export async function listProjects(): Promise<Project[]> {
  const r = await apiGet<ProjectsResponse>('/v1/projects');
  return r.projects ?? [];
}

export async function createProject(name: string, description = '', defaultRoot = ''): Promise<Project> {
  const r = await apiPost<ProjectResponse>('/v1/projects', {
    name,
    description,
    default_root: defaultRoot
  });
  return r.project;
}
