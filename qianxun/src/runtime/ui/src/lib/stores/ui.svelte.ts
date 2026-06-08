// Stage 9c — UI Store (临时 stub, 待 responsive sibling agent 实现完整)
// 跟 docs/30_子项目规划/01b-daemon-web-console.md §5 Stage 7c Web 响应式 一致
//
// 当前: 提供最小 reactive state + 兄弟 agent 期望的方法 (openSidebar / closeSidebar /
// toggleSidebar / autoCloseOnNav) 让 Sidebar / +layout 能编译.
//
// 完整版 (含 drawer breakpoint 同步 + focus trap + escape 关闭) 留 sibling 接管.

class UiStore {
	#sidebarOpen = $state(false);
	#initialized = $state(false);

	get sidebarOpen(): boolean {
		return this.#sidebarOpen;
	}

	get initialized(): boolean {
		return this.#initialized;
	}

	init(): void {
		if (this.#initialized) return;
		this.#initialized = true;
	}

	toggleSidebar(): void {
		this.#sidebarOpen = !this.#sidebarOpen;
	}

	openSidebar(): void {
		this.#sidebarOpen = true;
	}

	closeSidebar(): void {
		this.#sidebarOpen = false;
	}

	/// 路由变化时自动关闭 (sibling 期望在 +layout.svelte 调)
	autoCloseOnNav(): void {
		this.#sidebarOpen = false;
	}
}

export const uiStore = new UiStore();
