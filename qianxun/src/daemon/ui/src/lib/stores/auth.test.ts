// ──────────────────────────────────────────────────────────────────────────
// Stage 7a — authStore 单元测试
// 覆盖: init / setToken / clear / 401 → 清 token
// ──────────────────────────────────────────────────────────────────────────

import { describe, it, expect, beforeEach } from 'vitest';
import { authStore } from './auth.svelte';

describe('authStore', () => {
	beforeEach(() => {
		localStorage.clear();
		authStore.clear();
	});

	it('init: localStorage 有 token → 加载到内存', () => {
		localStorage.setItem('qianxun_admin_token', 'stored-token');
		authStore.init();
		expect(authStore.token).toBe('stored-token');
		expect(authStore.isAuthenticated).toBe(true);
	});

	it('init: localStorage 空 → 仍初始化, token=null', () => {
		authStore.init();
		expect(authStore.token).toBeNull();
		expect(authStore.isAuthenticated).toBe(false);
		expect(authStore.initialized).toBe(true);
	});

	it('setToken: 写内存 + localStorage', () => {
		authStore.setToken('abc123');
		expect(authStore.token).toBe('abc123');
		expect(localStorage.getItem('qianxun_admin_token')).toBe('abc123');
	});

	it('clear: 清内存 + localStorage', () => {
		authStore.setToken('xyz');
		authStore.clear();
		expect(authStore.token).toBeNull();
		expect(localStorage.getItem('qianxun_admin_token')).toBeNull();
	});

	it('isAuthenticated: 空 token 也算未登录', () => {
		authStore.setToken('');
		expect(authStore.isAuthenticated).toBe(false);
	});
});
