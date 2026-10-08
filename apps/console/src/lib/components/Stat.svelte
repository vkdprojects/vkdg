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
  <div class="stat-value" data-tone={tone}>
    <span class="stat-num">{value}</span>{#if unit}<span class="stat-unit">{unit}</span>{/if}
  </div>
</div>

<style>
  .stat { display: flex; flex-direction: column; gap: var(--space-2); }
  .stat-top { display: flex; align-items: center; justify-content: space-between; }
  .stat-label { font-size: var(--text-sm); font-weight: var(--weight-medium); color: var(--text-2); }
  .stat-icon {
    display: grid; place-items: center; width: var(--control-h-sm); height: var(--control-h-sm);
    border-radius: var(--radius-sm); background: var(--accent-subtle); color: var(--accent);
  }
  .stat-value {
    display: flex; align-items: baseline; gap: var(--space-2); flex-wrap: wrap;
    font-family: var(--font-dot); font-size: var(--text-hero); font-weight: var(--weight-dot);
    letter-spacing: var(--tracking-dot); color: var(--text-1); line-height: var(--leading-tight);
  }
  .stat-value[data-tone='success'] { color: var(--success); }
  .stat-value[data-tone='warning'] { color: var(--warning); }
  .stat-value[data-tone='danger'] { color: var(--danger); }
  .stat-unit { font-family: var(--font-mono); font-size: var(--text-lg); font-weight: var(--weight-medium); color: var(--text-3); letter-spacing: 0; }
</style>
