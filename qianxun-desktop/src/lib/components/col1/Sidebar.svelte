<script lang="ts">
	import NewTaskButton from './NewTaskButton.svelte';
	import ScheduledTaskSection from './ScheduledTaskSection.svelte';
	import TaskHistorySection from './TaskHistorySection.svelte';
	import ProjectSection from './ProjectSection.svelte';
	import AgentTeamSection from './AgentTeamSection.svelte';
	import SidebarFooter from './SidebarFooter.svelte';
</script>

<aside class="h-full flex flex-col bg-zinc-100 dark:bg-zinc-900 text-zinc-900 dark:text-zinc-100">
	<NewTaskButton />
	<div class="flex-1 overflow-y-auto py-2 space-y-3">
		<ScheduledTaskSection />
		<TaskHistorySection />
		<ProjectSection />
		<AgentTeamSection />
	</div>
	<SidebarFooter />
</aside>
