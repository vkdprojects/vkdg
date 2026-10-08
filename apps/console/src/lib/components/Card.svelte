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
    background: var(--glass-bg);
    -webkit-backdrop-filter: var(--glass-blur);
    backdrop-filter: var(--glass-blur);
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-lg);
    box-shadow: var(--glass-shadow);
  }
</style>
