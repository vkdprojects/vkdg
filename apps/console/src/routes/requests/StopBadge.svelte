<script lang="ts">
  interface Props {
    reason: string;
  }

  let { reason }: Props = $props();
</script>

<span class="stop-badge stop-badge--{reason}">{reason}</span>

<style>
  .stop-badge {
    display: inline-block;
    font-size: var(--text-2xs);
    font-weight: var(--weight-medium);
    padding: var(--space-0) var(--space-2);
    border-radius: var(--radius-full);
    white-space: nowrap;
    background: var(--bg-inset);
    border: var(--border-w) solid var(--border);
    color: var(--text-2);
  }

  .stop-badge--end_turn {
    background: var(--success-subtle);
    border: var(--border-w) solid color-mix(in oklch, var(--success) 28%, transparent);
    color: var(--success);
  }

  .stop-badge--max_tokens {
    background: var(--warning-subtle);
    border: var(--border-w) solid color-mix(in oklch, var(--warning) 28%, transparent);
    color: var(--warning);
  }

  .stop-badge--tool_use {
    background: var(--accent-subtle);
    border: var(--border-w) solid color-mix(in oklch, var(--accent) 28%, transparent);
    color: var(--accent);
  }
</style>
