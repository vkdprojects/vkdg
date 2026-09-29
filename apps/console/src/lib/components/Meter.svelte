<script lang="ts">
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
  }

  let {
    value,
    limit = null,
    label,
    valueText,
    unlimitedText = 'no limit configured',
    tone = 'accent',
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
  <div class="meter-track" role="progressbar" aria-valuenow={ratio !== null ? Math.round(ratio * 100) : undefined} aria-valuemin={0} aria-valuemax={100}>
    {#if ratio !== null}
      <div class="meter-fill" data-tone={effectiveTone} style="width: {ratio * 100}%"></div>
    {/if}
  </div>
  <div class="meter-foot">
    <span class="meter-value mono">{valueText ?? value.toLocaleString()}</span>
    {#if ratio === null}
      <span class="meter-unlimited">{unlimitedText}</span>
    {:else}
      <span class="meter-pct mono" data-tone={effectiveTone}>{Math.round(ratio * 100)}%</span>
    {/if}
  </div>
</div>

<style>
  .meter {
    display: flex;
    flex-direction: column;
    gap: 0.375rem;
  }

  .meter-label {
    font-size: var(--text-2xs);
    font-weight: 600;
    color: var(--text-3);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .meter-track {
    position: relative;
    height: 6px;
    border-radius: var(--radius-sm);
    background: var(--bg-inset);
    border: 1px solid var(--border);
    overflow: hidden;
  }

  .meter-fill {
    position: absolute;
    inset: 0;
    width: 0%;
    border-radius: var(--radius-sm);
    background: var(--accent);
    transition: width 0.2s ease;
  }
  .meter-fill[data-tone='success'] { background: var(--success); }
  .meter-fill[data-tone='warning'] { background: var(--warning); }
  .meter-fill[data-tone='danger'] { background: var(--danger); }

  .meter-foot {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.5rem;
    font-size: var(--text-xs);
  }

  .meter-value {
    color: var(--text-1);
  }

  .meter-unlimited {
    color: var(--text-3);
    font-style: italic;
  }

  .meter-pct {
    color: var(--text-2);
    font-weight: 600;
  }
  .meter-pct[data-tone='warning'] { color: var(--warning); }
  .meter-pct[data-tone='danger'] { color: var(--danger); }
</style>
