<script lang="ts">
  import { formatNumber } from '$lib/format.js';
  /**
   * Usage/limit gauge. Honest by construction: pass `limit={null}` (absent
   * limit) and it renders "no limit configured" text instead of a fake bar —
   * never draw a full or empty bar for a quantity that has no ceiling.
   */
  interface Props {
    /** Current value, e.g. tokens consumed this month. */
    value: number;
    /** Ceiling; null/undefined = unrestricted. */
    limit?: number | null;
    /** Label shown above the bar, e.g. "Tokens this month". */
    label?: string;
    /** Pre-formatted value text, e.g. "482.3K / 1M". Falls back to raw numbers. */
    valueText?: string;
    /** Text shown when limit is absent. */
    unlimitedText?: string;
    tone?: 'accent' | 'success' | 'warning' | 'danger';
    /** Accessible name when no visible label is rendered. */
    ariaLabel?: string;
    /** Bar only: hide the value/percent footer (caller shows the number). */
    bare?: boolean;
  }

  let {
    value,
    limit = null,
    label,
    valueText,
    unlimitedText = 'no limit configured',
    tone = 'accent',
    ariaLabel,
    bare = false,
  }: Props = $props();

  const ratio = $derived(limit && limit > 0 ? Math.min(value / limit, 1) : null);
  // Escalate tone automatically as headroom shrinks, unless the caller pins one.
  const effectiveTone = $derived.by(() => {
    if (ratio === null) return tone;
    if (ratio >= 0.95) return 'danger';
    if (ratio >= 0.8) return 'warning';
    return tone;
  });
</script>

<div class="meter">
  {#if label}<div class="meter-label">{label}</div>{/if}
  <div class="meter-track" role="progressbar" aria-label={ariaLabel ?? label} aria-valuenow={ratio !== null ? Math.round(ratio * 100) : undefined} aria-valuemin={0} aria-valuemax={100}>
    {#if ratio !== null}
      <div class="meter-fill" class:meter-fill-min={ratio > 0} data-tone={effectiveTone} style="width: {ratio * 100}%"></div>
    {/if}
  </div>
  {#if !bare}
  <div class="meter-foot">
    <span class="meter-value mono">{valueText ?? formatNumber(value)}</span>
    {#if ratio === null}
      <span class="meter-unlimited">{unlimitedText}</span>
    {:else}
      <span class="meter-pct mono" data-tone={effectiveTone}>{Math.round(ratio * 100)}%</span>
    {/if}
  </div>
  {/if}
</div>

<style>
  .meter { display: flex; flex-direction: column; gap: var(--space-1); min-width: 0; }

  .meter-label { font-size: var(--text-xs); font-weight: var(--weight-medium); color: var(--text-2); }

  .meter-track {
    position: relative; height: var(--meter-h); border-radius: var(--radius-full);
    background: var(--bg-inset);
    box-shadow: inset 0 0 0 var(--border-w) var(--border);
    overflow: hidden;
  }

  .meter-fill {
    position: absolute; inset: 0 auto 0 0; width: 0%;
    border-radius: var(--radius-full);
    background: var(--accent);
    transition: width var(--dur-3) var(--ease-out), background var(--dur-2) var(--ease-out);
  }
  /* A pinned non-accent tone and threshold escalation both use flat semantic colors. */
  .meter-fill[data-tone='success'] { background: var(--success); }
  .meter-fill[data-tone='warning'] { background: var(--warning); }
  .meter-fill[data-tone='danger'] { background: var(--danger); }
  /* Distinguish "used a little" from "used nothing": floor the visible fill
     so a 1% ratio isn't an invisible sliver, while ratio===0 stays truly empty. */
  .meter-fill-min { min-width: var(--meter-h); }

  .meter-foot { display: flex; align-items: baseline; justify-content: space-between; gap: var(--space-2); font-size: var(--text-xs); min-width: 0; }
  .meter-value { color: var(--text-1); overflow-wrap: anywhere; }
  .meter-unlimited { color: var(--text-3); font-style: italic; }
  .meter-pct { color: var(--text-2); font-weight: var(--weight-semibold); flex-shrink: 0; }
  .meter-pct[data-tone='warning'] { color: var(--warning); }
  .meter-pct[data-tone='danger'] { color: var(--danger); }

  @media (prefers-reduced-motion: reduce) { .meter-fill { transition: none; } }
</style>
