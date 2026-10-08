<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    title: string;
    description?: string;
    action?: Snippet;
    icon?: Snippet;
  }

  let { title, description, action, icon }: Props = $props();
</script>

<div class="empty-state">
  {#if icon}
    <div class="icon" aria-hidden="true">{@render icon()}</div>
  {/if}
  <p class="title">{title}</p>
  {#if description}
    <p class="desc">{description}</p>
  {/if}
  {#if action}
    <div class="action">{@render action()}</div>
  {/if}
</div>

<style>
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-7) var(--space-5);
    text-align: center;
    border: 1px dashed var(--border-strong);
    border-radius: var(--radius-lg);
    background: color-mix(in oklch, var(--bg-surface) 60%, transparent);
  }

  .icon {
    display: grid;
    place-items: center;
    width: var(--space-7);
    height: var(--space-7);
    border-radius: var(--radius-full);
    background: var(--accent-subtle);
    box-shadow: 0 0 0 var(--space-2) color-mix(in oklch, var(--accent) 6%, transparent);
    color: var(--accent);
    margin-bottom: var(--space-2);
  }

  .title {
    font-size: var(--text-md);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--tracking-tight);
    color: var(--text-1);
    margin: 0;
  }

  .desc {
    font-size: var(--text-sm);
    color: var(--text-3);
    margin: 0;
    max-width: 36ch;
  }

  .action {
    margin-top: var(--space-3);
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: var(--space-2);
  }
</style>
