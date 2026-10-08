<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    /** Element to render; `label` for a selectable radio card, `article` for a record. */
    as?: 'article' | 'label';
    class?: string;
    children: Snippet;
  }

  let { as = 'article', class: className = '', children }: Props = $props();
</script>

<svelte:element this={as} class="surface-card {className}">
  {@render children()}
</svelte:element>

<style>
  .surface-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-5);
    background: var(--bg-surface);
    border: var(--border-w) solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-1);
    min-width: 0;
  }
</style>
