<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    padding?: string;
    children?: Snippet;
  }

  let { padding = '1.25rem', children }: Props = $props();
</script>

<div class="card" style="padding: {padding}">
  {#if children}{@render children()}{/if}
</div>

<style>
  .card {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background-image: linear-gradient(180deg, var(--bg-elevated) 0%, var(--bg-surface) 100%);
  }
</style>
