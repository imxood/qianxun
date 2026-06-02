<script lang="ts">
	// Token 配置弹窗 — 首次访问 / 401 时由 +layout 触发.
	// 用户输入 admin token, 写入 authStore + localStorage.

	import Dialog from '$lib/components/ui/dialog/Dialog.svelte';
	import DialogFooter from '$lib/components/ui/dialog/DialogFooter.svelte';
	import DialogBody from '$lib/components/ui/dialog/DialogBody.svelte';
	import Input from '$lib/components/ui/input/Input.svelte';
	import Button from '$lib/components/ui/button/Button.svelte';
	import Label from '$lib/components/ui/label/Label.svelte';
	import { authStore } from '$lib/stores/auth.svelte';

	type Props = {
		open: boolean;
		onOpenChange: (open: boolean) => void;
	};

	let { open, onOpenChange }: Props = $props();

	let token = $state('');
	let error = $state<string | null>(null);
	let testing = $state(false);

	async function handleSubmit() {
		const t = token.trim();
		if (!t) {
			error = '请输入 token';
			return;
		}
		error = null;
		testing = true;
		authStore.setToken(t);
		// 调一次 /v1/system/status 验证 token 是否有效
		try {
			const r = await fetch('/v1/system/status', {
				headers: { Authorization: `Bearer ${t}` }
			});
			if (r.ok) {
				token = '';
				onOpenChange(false);
			} else if (r.status === 401) {
				error = 'Token 无效 (401)';
				authStore.clear();
			} else {
				error = `验证失败: HTTP ${r.status}`;
			}
		} catch (e) {
			error = e instanceof Error ? e.message : '网络错误';
		} finally {
			testing = false;
		}
	}

	function onCancel() {
		token = '';
		error = null;
		onOpenChange(false);
	}
</script>

<Dialog
	{open}
	{onOpenChange}
	title="配置 Admin Token"
	description="首次访问需粘贴 daemon 启动时打印到 stderr 的 token."
>
	<DialogBody>
		<form
			onsubmit={(e) => {
				e.preventDefault();
				void handleSubmit();
			}}
		>
			<div class="flex flex-col gap-1.5">
				<Label for="token-input">Admin Token</Label>
				<Input
					id="token-input"
					type="password"
					placeholder="Bearer token (粘 daemon stderr 输出)"
					bind:value={token}
					disabled={testing}
					autocomplete="off"
					autofocus
					data-testid="token-input"
				/>
				{#if error}
					<p class="text-destructive text-xs" data-testid="token-error">{error}</p>
				{/if}
				<p class="text-muted-foreground text-xs">
					存 localStorage, 后续请求自动带 <code>Authorization: Bearer</code>.
				</p>
			</div>
		</form>
	</DialogBody>
	<DialogFooter>
		<Button variant="outline" type="button" onclick={onCancel} disabled={testing}>
			取消
		</Button>
		<Button
			type="button"
			onclick={handleSubmit}
			disabled={testing || !token.trim()}
			data-testid="token-submit"
		>
			{testing ? '验证中…' : '保存并验证'}
		</Button>
	</DialogFooter>
</Dialog>
