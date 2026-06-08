// qianxun-desktop/src/lib/mock/seed.ts
// 一键 seed 所有 mock 数据 (mock 阶段)

import { mockProjects } from './projects';
import { mockSessions } from './sessions';
import { mockPlans, mockChangedFiles } from './plans';
import { mockSubSessions } from './sub_sessions';
import { mockMessages, mockMessagesDarkMode } from './messages';
import { mockExperience } from './experience';
import { mockScheduledTasks } from './scheduled';

import type { Project, Session, Plan, SubSession, Message, ProjectExperience, ScheduledTask, ChangedFile } from '$lib/types/entity';
import type { Toast } from '$lib/types/ui';

export interface MockSeed {
	projects: Project[];
	sessions: Session[];
	plans: Plan[];
	changed_files: ChangedFile[];
	sub_sessions: SubSession[];
	messages: Message[];
	messages_dark_mode: Message[];
	experience: ProjectExperience[];
	scheduled: ScheduledTask[];
	started_toasts: Toast[];
}

export function buildSeed(): MockSeed {
	return {
		projects: structuredClone(mockProjects),
		sessions: structuredClone(mockSessions),
		plans: structuredClone(mockPlans),
		changed_files: structuredClone(mockChangedFiles),
		sub_sessions: structuredClone(mockSubSessions),
		messages: structuredClone(mockMessages),
		messages_dark_mode: structuredClone(mockMessagesDarkMode),
		experience: structuredClone(mockExperience),
		scheduled: structuredClone(mockScheduledTasks),
		started_toasts: [],
	};
}
