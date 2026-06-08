<script lang="ts">
	// Stage 10a — Password 登录弹窗 (替代 Stage 7a 的 TokenDialog)
	//
	// 用户输入 admin 密码 → 调 authStore.login(pw) → 拿 {token, exp} → 存 localStorage.
	// 错误 (401) 时显示 "密码错误".
	//
	// 这个 dialog 是 first-time / 401 触发弹, 由 +layout.svelte 控制 open 状态.
	// 不可关闭 (close 也会 clear token). Cancel 按钮: 仅在已是已登录状态可点 (会调 logout).

	import Dialog from '$lib/components/ui/dialog/Dialog.svelte';
	import DialogFooter from '$lib/components/ui/dialog/DialogFooter.svelte';
	import DialogBody from '$lib/components/ui/dialog/DialogBody.svelte';
	import Input from '$lib/components/ui/input/Input.svelte';
	import Button from '$lib/components/ui/button/Button.svelte';
	import Label from '$lib/components/ui/label/Label.svelte';
	import { authStore } from '$lib/stores/auth.svelte';
	import { t } from '$lib/i18n';

	type Props = {
		open: boolean;
		onOpenChange: (open: boolean) => void;
	};

	let { open, onOpenChange }: Props = $props();

	let password = $state('');
	let error = $state<string | null>(null);
	let submitting = $state(false);

	async function handleSubmit() {
		const pw = password;
		if (!pw) {
			error = t('auth.login.error_empty');
			return;
		}
		error = null;
		submitting = true;
		try {
			await authStore.login(pw);
			password = '';
			onOpenChange(false);
		} catch (e) {
			// 区分 401 (密码错) vs 其它
			const isAuthErr = e instanceof Error && (
				e.name === 'ApiError' ||
				e.message.includes('401') ||
				e.message.includes('Invalid password') ||
				e.message.includes('invalid_credentials')
			);
			if (isAuthErr) {
				error = t('auth.login.error');
			} else {
				error = e instanceof Error ? e.message : t('common.error');
			}
		} finally {
			submitting = false;
		}
	}

	async function handleCancel() {
		password = '';
		error = null;
		// 已登录才能 cancel — 清 token 走 logout 流程
		if (authStore.isAuthenticated) {
			await authStore.logout();
		}
		onOpenChange(false);
	}
</script>

<Dialog
	{open}
	{onOpenChange}
	title={t('auth.login.title')}
	description={t('auth.login.desc')}
>
	<DialogBody>
		<form
			onsubmit={(e) => {
				e.preventDefault();
				void handleSubmit();
			}}
		>
			<div class="flex flex-col gap-1.5">
				<Label for="password-input">{t('auth.login.password')}</Label>
				<Input
					id="password-input"
					type="password"
					placeholder={t('auth.login.password_placeholder')}
					bind:value={password}
					disabled={submitting}
					autocomplete="current-password"
					autofocus
					data-testid="password-input"
				/>
				{#if error}
					<p class="text-destructive text-xs" data-testid="password-error">{error}</p>
				{/if}
				<p class="text-muted-foreground text-xs">
					{t('auth.login.hint')}
				</p>
			</div>
		</form>
	</DialogBody>
	<DialogFooter>
		<Button variant="outline" type="button" onclick={handleCancel} disabled={submitting}>
			{authStore.isAuthenticated ? t('auth.logout') : t('common.cancel')}
		</Button>
		<Button
			type="button"
			onclick={handleSubmit}
			disabled={submitting || !password}
			data-testid="password-submit"
		>
			{submitting ? t('auth.login.submitting') : t('auth.login.submit')}
		</Button>
	</DialogFooter>
</Dialog>
