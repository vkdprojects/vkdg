<script lang="ts">
  interface Props {
    /** `[phase, epoch_ms]` pairs in pipeline order. */
    transitions: [string, number][];
  }

  let { transitions }: Props = $props();

  const upstreamOpenTs = $derived(transitions.find(([p]) => p === 'UpstreamOpen')?.[1] ?? null);
</script>

<ol class="timeline" aria-label="Pipeline phase timestamps">
  {#each transitions as [phase, ts], i (phase)}
    {@const deltaMs = i === 0 ? null : ts - transitions[i - 1][1]}
    {@const isUpstreamOpen = phase === 'UpstreamOpen'}
    {@const isTtfb = phase === 'Committed' && i > 0 && upstreamOpenTs != null}
    {@const ttfbMs = isTtfb && upstreamOpenTs != null ? ts - upstreamOpenTs : null}
    {@const gatewayMs = isUpstreamOpen && i > 0 ? ts - transitions[0][1] : null}
    <li class="timeline-item" class:timeline-ttfb={isTtfb} class:timeline-overhead={isUpstreamOpen}>
      <span class="timeline-phase">{phase}</span>
      <span class="timeline-ts mono">{new Date(ts).toISOString().slice(11, 23)}</span>
      {#if deltaMs != null}
        <span class="timeline-delta">+{deltaMs}ms</span>
      {/if}
      {#if gatewayMs != null}
        <span class="timeline-label timeline-label--overhead">gateway overhead: {gatewayMs}ms</span>
      {/if}
      {#if ttfbMs != null}
        <span class="timeline-label timeline-label--ttfb">TTFB: {ttfbMs}ms</span>
      {/if}
    </li>
  {/each}
</ol>

<style>
  .timeline {
    list-style: none;
    padding: 0 0 0 var(--space-4);
    margin: 0;
    border-left: var(--focus-w) solid var(--border-strong);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .timeline-item {
    position: relative;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-2);
  }

  .timeline-item::before {
    content: '';
    position: absolute;
    left: calc(-1 * var(--space-4) - var(--dot-size) / 2 - var(--focus-w) / 2);
    top: 50%;
    width: var(--dot-size);
    height: var(--dot-size);
    margin-top: calc(-0.5 * var(--dot-size));
    border-radius: var(--radius-full);
    background: var(--bg-surface);
    border: var(--focus-w) solid var(--border-strong);
  }

  .timeline-item.timeline-ttfb,
  .timeline-item.timeline-overhead {
    color: var(--text-1);
    font-weight: var(--weight-medium);
  }

  .timeline-item.timeline-ttfb::before,
  .timeline-item.timeline-overhead::before {
    border-color: var(--accent);
    background: var(--accent);
  }

  .timeline-phase {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-2);
    min-width: calc(var(--space-8) * 1.5);
  }

  .timeline-ts {
    font-size: var(--text-xs);
    color: var(--text-3);
  }

  .timeline-delta {
    font-size: var(--text-xs);
    background: var(--bg-inset);
    border: var(--border-w) solid var(--border);
    color: var(--text-2);
    padding: 0 var(--space-1);
    border-radius: var(--radius-sm);
    font-family: var(--font-mono);
  }

  .timeline-label {
    display: inline-block;
    font-size: var(--text-2xs);
    font-weight: var(--weight-semibold);
    padding: var(--space-0) var(--space-2);
    border-radius: var(--radius-full);
    white-space: nowrap;
  }

  .timeline-label--overhead {
    background: var(--warning-subtle);
    border: var(--border-w) solid color-mix(in oklch, var(--warning) 28%, transparent);
    color: var(--warning);
  }

  .timeline-label--ttfb {
    background: var(--accent-subtle);
    border: var(--border-w) solid color-mix(in oklch, var(--accent) 28%, transparent);
    color: var(--accent);
  }
</style>
