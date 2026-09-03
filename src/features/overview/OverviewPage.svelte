<script lang="ts">
  import { roadmap } from './roadmap';
</script>

<section class="mx-auto max-w-4xl space-y-6">
  <header class="space-y-2">
    <h1 class="text-lg font-semibold">概览</h1>
    <p class="text-sm leading-6 text-muted">v0.1 已交付全部核心能力。</p>
  </header>

  <div class="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
    {#each roadmap as item (item.title)}
      <article class="rounded-lg border border-line bg-card p-4">
        <div class="flex items-center justify-between">
          <h2 class="text-sm font-medium">{item.title}</h2>
          <span class="rounded bg-accent-soft px-1.5 py-0.5 text-xs text-fg">{item.version}</span>
        </div>
        <p class="mt-2 text-xs leading-5 text-muted">{item.summary}</p>
      </article>
    {/each}
  </div>
</section>
