// qianxun-desktop/src/lib/utils/stream.ts
// Mock 流式输出 (模拟 LLM 文本流)

import type { Message } from '$lib/types/entity';

const MOCK_RESPONSES: Record<string, string[]> = {
	default: [
		'好的, 让我看看你的需求.',
		'\n\n我会先分析一下当前的项目结构,',
		'然后给出一个具体的实施方案.',
		'\n\n',
		'首先, ',
		'我们需要确认几个关键点:',
		'\n- 当前项目的技术栈',
		'\n- 已有代码的结构',
		'\n- 目标实现的复杂度',
		'\n\n',
		'基于这些信息, 我会决定是否需要拉一个 Plan 来拆分子任务.',
		'\n\n',
		'让我先看一下相关的文件...',
	],
};

export async function streamMock(
	_msg: Message,
	_prompt: string,
	onUpdate: (content: string) => void,
): Promise<void> {
	// 简单 mock: 拼接 default 响应, 每 50ms append 一段
	const chunks = MOCK_RESPONSES.default;
	let content = '';
	for (let i = 0; i < chunks.length; i++) {
		content += chunks[i];
		onUpdate(content);
		await sleep(50 + Math.random() * 30);
	}
}

function sleep(ms: number) {
	return new Promise((r) => setTimeout(r, ms));
}
