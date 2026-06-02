// ───────────────────────────────────────────────────────────────────────────
// TeamSwitcher.svelte — Stage 5 §9.1 团队切换器回调测试
// 与 docs/30_子项目规划/03-tauri-desktop.md §9.1 一致
//
// 测试: 选 <select> 触发 change 事件, onSelect 回调被调用且参数正确
// 渲染方式: Svelte 5 `mount` + jsdom (与 vitest.config.ts environment 匹配)
// ───────────────────────────────────────────────────────────────────────────

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { mount, unmount } from "svelte";
import type { Team } from "$lib/types/ipc";
import TeamSwitcher from "$lib/components/team/TeamSwitcher.svelte";

const sampleTeams: Team[] = [
	{
		id: "team_1",
		name: "千寻 R&D",
		created_at: "2026-06-01T08:00:00Z",
		members: [
			{
				user_id: "u_1",
				display_name: "maxu",
				role: "owner",
				joined_at: "2026-06-01T08:00:00Z",
			},
		],
	},
	{
		id: "team_2",
		name: "个人实验",
		created_at: "2026-06-01T09:00:00Z",
		members: [
			{
				user_id: "u_2",
				display_name: "alice",
				role: "owner",
				joined_at: "2026-06-01T09:00:00Z",
			},
			{
				user_id: "u_3",
				display_name: "bob",
				role: "viewer",
				joined_at: "2026-06-01T09:30:00Z",
			},
		],
	},
];

describe("TeamSwitcher (Stage 5 §9.1)", () => {
	let target: HTMLDivElement;
	let component: ReturnType<typeof mount>;

	beforeEach(() => {
		target = document.createElement("div");
		document.body.appendChild(target);
	});

	afterEach(() => {
		if (component) {
			unmount(component);
			component = undefined as unknown as ReturnType<typeof mount>;
		}
		target.remove();
	});

	it("选 select 后 onSelect 回调被触发, 参数 = 新值", () => {
		const onSelect = vi.fn();
		component = mount(TeamSwitcher, {
			target,
			props: { teams: sampleTeams, activeTeamId: "team_1", onSelect },
		});

		// 渲染验证: <select> + 每个 team 一个 <option>
		const select = target.querySelector("select");
		expect(select).toBeTruthy();
		const options = target.querySelectorAll("option");
		expect(options).toHaveLength(2);
		expect(options[0]?.value).toBe("team_1");
		expect(options[1]?.value).toBe("team_2");
		// option 文本含 "成员"
		expect(options[0]?.textContent).toContain("1 成员");
		expect(options[1]?.textContent).toContain("2 成员");

		// 模拟用户切换
		select!.value = "team_2";
		select!.dispatchEvent(new Event("change", { bubbles: true }));

		expect(onSelect).toHaveBeenCalledTimes(1);
		expect(onSelect).toHaveBeenCalledWith("team_2");
	});

	it("teams 为空时显示 '暂无团队', 不渲染 select", () => {
		const onSelect = vi.fn();
		component = mount(TeamSwitcher, {
			target,
			props: { teams: [], onSelect },
		});

		const select = target.querySelector("select");
		expect(select).toBeNull();
		const empty = target.textContent;
		expect(empty).toContain("暂无团队");
		// 不会调回调
		expect(onSelect).not.toHaveBeenCalled();
	});
});
