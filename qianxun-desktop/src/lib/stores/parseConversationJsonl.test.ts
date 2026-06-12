// ───────────────────────────────────────────────────────────────────────────
// parseConversationJsonl — 批次 1.5 (解析/数据一致性 验证)
//
// 测试覆盖 (2026-06-12 评审修复):
//   1. parse_handles_corrupt_line: 损坏行 try/catch skip, 不阻断整体 (评审 #2)
//   2. parse_handles_system_header_with_space: serde 输出带空格也能匹配 (评审 #1)
//   3. parse_preserves_created_at_via_outer_idempotency: 后端无 created_at, 用 now 兜底
//   4. parse_fallbacks_to_now_when_no_created_at: 兜底行为本身
// ───────────────────────────────────────────────────────────────────────────

import { describe, it, expect } from "vitest";
import { parseConversationJsonl } from "$lib/stores/session.svelte";

const SID = "sess_test_001";

describe("parseConversationJsonl (批次 1.5)", () => {
	it("parse_handles_corrupt_line: 损坏行静默 skip, 后续行继续解析", () => {
		const jsonl = [
			'{"type": "system", "prompt": "You are helpful"}', // 合法 system header
			"not a json line at all, just garbage",            // 损坏行 → skip
			'{"User": {"id": "m1", "content": [{"type": "text", "text": "hi"}]}}',
			"{ broken: json,",                                  // 损坏行 → skip
			'{"Assistant": {"id": "m2", "content": [{"type": "text", "text": "hello"}]}}',
		].join("\n");

		const result = parseConversationJsonl(jsonl, SID);

		// 期望: 跳过 system header, 跳过 2 个损坏行, 拿到 2 条 user/assistant
		expect(result).toHaveLength(2);
		expect(result[0]?.role).toBe("user");
		expect(result[0]?.content).toBe("hi");
		expect(result[1]?.role).toBe("assistant");
		expect(result[1]?.content).toBe("hello");
	});

	it("parse_handles_system_header_with_space: serde 输出带空格也能跳过", () => {
		// 真实场景: serde_json::to_string 序列化为 `{"type": "system"` (key 跟 value 间有空格)
		// 老实现 startsWith('{"type":"system"') 不带空格会漏, 改成宽松匹配.
		const withSpace = '{"type": "system", "prompt": "You are helpful"}';
		const withoutSpace = '{"type":"system","prompt":"You are helpful"}';

		const r1 = parseConversationJsonl(withSpace, SID);
		const r2 = parseConversationJsonl(withoutSpace, SID);

		// 两种 header 格式都被识别为 system 行 → 跳过
		expect(r1).toHaveLength(0);
		expect(r2).toHaveLength(0);
	});

	it("parse_fallbacks_to_now_when_no_created_at: 后端 Message 无 created_at 字段", () => {
		// 后端 qianxun-core/src/agent/message.rs Message struct 没 created_at,
		// 序列化时不携带, TS 端用 now 兜底. 这是新发现 A 的修复点.
		const jsonl = JSON.stringify({
			User: {
				id: "m1",
				content: [{ type: "text", text: "hi" }],
				// 注意: 没有 created_at 字段
			},
		});

		const before = new Date().toISOString();
		const result = parseConversationJsonl(jsonl, SID);
		const after = new Date().toISOString();

		expect(result).toHaveLength(1);
		expect(result[0]?.id).toBe("m1");
		expect(result[0]?.role).toBe("user");
		// created_at 应是 now 兜底, 在 [before, after] 区间内
		const created = result[0]?.created_at ?? "";
		expect(created >= before).toBe(true);
		expect(created <= after).toBe(true);
	});

	it("parse_skips_unknown_tags: 非 User/Assistant tag 行静默 skip", () => {
		const jsonl = [
			'{"Type": {"id": "m1", "content": []}}', // 错误大小写, 跳过
			'{"System": {"id": "m2", "content": []}}', // 错误 tag, 跳过
			'{"User": {"id": "m3", "content": [{"type": "text", "text": "valid"}]}}',
		].join("\n");

		const result = parseConversationJsonl(jsonl, SID);

		expect(result).toHaveLength(1);
		expect(result[0]?.id).toBe("m3");
		expect(result[0]?.role).toBe("user");
	});
});
