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
    <div class="icon">{@render icon()}</div>
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
    gap: 0.5rem;
    padding: 3rem 1.5rem;
    text-align: center;
  }

  .icon {
    color: var(--text-3);
    margin-bottom: 0.25rem;
  }

  .title {
    font-size: 0.9375rem;
    font-weight: 500;
    color: var(--text-2);
    margin: 0;
  }

  .desc {
    font-size: 0.8125rem;
    color: var(--text-3);
    margin: 0;
    max-width: 320px;
  }

  .action {
    margin-top: 0.5rem;
  }
</style>
