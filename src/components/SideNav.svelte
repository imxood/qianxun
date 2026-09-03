<script lang="ts">
  import { nav, type PageId } from '../stores/nav.svelte';

  const items: Array<{ id: PageId; label: string; icon: string }> = [
    { id: 'overview', label: '概览', icon: 'M3 12l9-8 9 8M5 10v10h5v-6h4v6h5V10' },
    {
      id: 'env',
      label: '环境',
      icon: 'M4 7h16M4 7a2 2 0 012-2h2m10 0a2 2 0 012 2M6 5v14m12-14v14M6 19h12',
    },
    {
      id: 'console',
      label: '控制台',
      icon: 'M4 5h16v14H4zM7 9l3 3-3 3m6 0h4',
    },
    {
      id: 'dsh',
      label: 'DSH',
      icon: 'M12 3l8 4.5v9L12 21l-8-4.5v-9zM12 12l8-4.5M12 12v9M12 12L4 7.5',
    },
    {
      id: 'search-files',
      label: '找文件',
      icon: 'M14 3a7 7 0 100 14 7 7 0 000-14zM20 20l-4.9-4.9M10 7h8M10 11h5',
    },
    {
      id: 'search-grep',
      label: '搜内容',
      icon: 'M4 6h16M4 12h16M4 18h10M17 18a2 2 0 104 0 2 2 0 00-4 0z',
    },
    {
      id: 'terminal',
      label: '终端',
      icon: 'M4 5h16v14H4zM7.5 9l3 3-3 3M12.5 15h4',
    },
    {
      id: 'remote',
      label: '远程',
      icon: 'M8 2h8a2 2 0 012 2v16a2 2 0 01-2 2H8a2 2 0 01-2-2V4a2 2 0 012-2zM10 18h4',
    },
    {
      id: 'notes',
      label: '笔记',
      icon: 'M7 3h10a2 2 0 012 2v14a2 2 0 01-2 2H7a2 2 0 01-2-2V5a2 2 0 012-2zM9 8h6M9 12h6M9 16h4',
    },
    {
      id: 'settings',
      label: '设置',
      icon: 'M12 8a4 4 0 100 8 4 4 0 000-8zM19 12l2-1-2-4-2 1-2-1V4h-2.2L12 6l-2.8-2H7v3l-2 1-2-1-2 4 2 1v2l-2 1 2 4 2-1 2 1v3h2.8L12 18l2.8 2H17v-3l2-1 2 1 2-4-2-1z',
    },
  ];
</script>

<nav class="flex w-16 shrink-0 flex-col gap-1 border-r border-line bg-surface p-1.5">
  {#each items as item (item.id)}
    {@const active = nav.page === item.id}
    <button
      class="group flex flex-col items-center gap-1 rounded-lg px-1 py-2 transition-colors {active
        ? 'bg-accent-soft'
        : 'hover:bg-accent-soft/60'}"
      aria-current={active ? 'page' : undefined}
      onclick={() => nav.go(item.id)}
    >
      <svg
        viewBox="0 0 24 24"
        class="size-5 shrink-0 {active ? 'text-accent' : 'text-muted group-hover:text-fg'}"
        fill="none"
        stroke="currentColor"
        stroke-width="1.6"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <path d={item.icon} />
      </svg>
      <span
        class="text-[11px] leading-none {active
          ? 'font-medium text-fg'
          : 'text-muted group-hover:text-fg'}"
      >
        {item.label}
      </span>
    </button>
  {/each}
</nav>
