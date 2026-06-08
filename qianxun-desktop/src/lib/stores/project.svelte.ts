// qianxun-desktop/src/lib/stores/project.svelte.ts
// Project store

import { buildSeed } from '$lib/mock/seed';
import type { Project } from '$lib/types/entity';

function createProjectStore() {
	const projects = $state<Project[]>(buildSeed().projects);

	return {
		get all() {
			return projects;
		},
		get(id: string): Project | undefined {
			return projects.find((p) => p.id === id);
		},
		byId(id: string): Project | undefined {
			return projects.find((p) => p.id === id);
		},
	};
}

export const projectStore = createProjectStore();
