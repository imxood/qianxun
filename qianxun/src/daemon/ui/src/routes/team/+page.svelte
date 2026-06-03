<script lang="ts">
	// /team — profiles (4 默认) + roles 表格 (2026-06-04 阶段 3)
	import { onMount } from 'svelte';
	import { User, Briefcase } from '@lucide/svelte';
	import { listProfiles, listRoles } from '$lib/api/kanban';
	import type { Profile, Role } from '$lib/types/kanban';

	let profiles = $state<Profile[]>([]);
	let roles = $state<Role[]>([]);
	let error = $state<string | null>(null);

	async function refresh() {
		error = null;
		try {
			[profiles, roles] = await Promise.all([listProfiles(), listRoles()]);
		} catch (e) {
			error = e instanceof Error ? e.message : '加载失败';
		}
	}

	onMount(refresh);
</script>

<svelte:head>
	<title>Team · 千寻</title>
</svelte:head>

<div class="flex flex-col gap-4 p-6">
	<header class="flex items-center gap-2">
		<User class="size-5" />
		<h1 class="text-lg font-semibold">Team</h1>
		<span class="text-muted-foreground text-xs">
			({profiles.length} profiles, {roles.length} roles)
		</span>
	</header>

	{#if error}
		<div class="rounded border border-red-300 bg-red-50 p-2 text-xs text-red-700" data-testid="error">
			{error}
		</div>
	{/if}

	<div class="grid grid-cols-1 gap-4 lg:grid-cols-2">
		<!-- Profiles -->
		<section class="bg-card rounded border p-4" data-testid="profiles-section">
			<header class="mb-3 flex items-center gap-2">
				<User class="size-4" />
				<h2 class="text-sm font-semibold">Profiles (执行单元)</h2>
			</header>
			{#if profiles.length === 0}
				<div class="text-muted-foreground py-4 text-center text-xs">暂无 profile</div>
			{:else}
				<div class="flex flex-col gap-2">
					{#each profiles as p (p.id)}
						<div class="rounded border bg-background/50 p-2" data-testid="profile-{p.id}">
							<div class="mb-1 flex items-center justify-between">
								<span class="font-mono text-xs font-semibold">{p.name}</span>
								<span class="text-muted-foreground text-[10px]">{p.kind}</span>
							</div>
							<div class="text-muted-foreground text-[10px]">
								working_dir: {p.working_dir} · max_turns: {p.max_turns}
							</div>
							<div class="text-muted-foreground text-[10px]">
								model: {p.model ?? '(default)'}
							</div>
						</div>
					{/each}
				</div>
			{/if}
		</section>

		<!-- Roles -->
		<section class="bg-card rounded border p-4" data-testid="roles-section">
			<header class="mb-3 flex items-center gap-2">
				<Briefcase class="size-4" />
				<h2 class="text-sm font-semibold">Roles (角色模板)</h2>
			</header>
			{#if roles.length === 0}
				<div class="text-muted-foreground py-4 text-center text-xs">暂无 role</div>
			{:else}
				<div class="flex flex-col gap-2">
					{#each roles as r (r.id)}
						<div class="rounded border bg-background/50 p-2" data-testid="role-{r.id}">
							<div class="mb-1 flex items-center justify-between">
								<span class="font-mono text-xs font-semibold">{r.name}</span>
								<span class="text-muted-foreground text-[10px]">→ {r.default_profile_id}</span>
							</div>
							<div class="text-muted-foreground mb-1 text-[10px]">{r.description}</div>
							<div class="text-muted-foreground line-clamp-2 text-[10px] italic">
								{r.instructions}
							</div>
						</div>
					{/each}
				</div>
			{/if}
		</section>
	</div>
</div>
