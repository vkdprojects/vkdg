<script lang="ts">
  import type { Snippet } from 'svelte';

  /** Stat tile: a labeled metric, mono-numeral, with an optional trailing unit or icon slot. */
  interface Props {
    label: string;
    value: string | number;
    unit?: string;
    tone?: 'default' | 'success' | 'warning' | 'danger';
    icon?: Snippet;
  }

  let { label, value, unit, tone = 'default', icon }: Props = $props();
</script>

<div class="stat">
  <div class="stat-top">
    <span class="stat-label">{label}</span>
    {#if icon}<span class="stat-icon">{@render icon()}</span>{/if}
  </div>
  <div class="stat-value mono" data-tone={tone}>
    {value}
    {#if unit}<span class="stat-unit">{unit}</span>{/if}
  </div>
</div>

<style>
  .stat {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .stat-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .stat-label {
    font-size: var(--text-2xs);
    font-weight: 600;
    color: var(--text-3);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .stat-icon {
    color: var(--text-3);
    display: inline-flex;
  }

  .stat-value {
    font-size: 1.5rem;
    font-weight: 600;
    color: var(--text-1);
    line-height: 1.1;
  }
  .stat-value[data-tone='success'] { color: var(--success); }
  .stat-value[data-tone='warning'] { color: var(--warning); }
  .stat-value[data-tone='danger'] { color: var(--danger); }

  .stat-unit {
    font-size: var(--text-sm);
    font-weight: 400;
    color: var(--text-3);
    margin-left: 0.25rem;
  }
</style>
