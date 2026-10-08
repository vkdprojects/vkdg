<script lang="ts">
  import { m } from '$lib/paraglide/messages.js';
  import { formatRelativeTime } from '$lib/format.js';
  import { remainingPercent, resetDelta } from '$lib/usage.js';
  import { usageLabels } from '$lib/usage-labels.js';
  import type { UsageWindow } from '$lib/api.js';
  import Meter from './Meter.svelte';

  /**
   * One bar per rate-limit window the upstream reported: used share, what is
   * left, and when it resets. A plan without a given window (e.g. no general
   * weekly limit) simply has no bar for it.
   */
  interface Props {
    windows: UsageWindow[];
  }

  let { windows }: Props = $props();

  function resetText(resetsAt: number | null): string | null {
    const delta = resetDelta(resetsAt, Date.now());
    return delta ? m.usage_resets({ when: formatRelativeTime(delta.n, delta.unit) }) : null;
  }
</script>

<div class="windows">
  {#each windows as w (w.kind)}
    {@const left = Math.round(remainingPercent(w.used_percent))}
    <div class="window" data-kind={w.kind}>
      <Meter
        label={usageLabels[w.kind]()}
        value={Math.min(100, Math.max(0, w.used_percent))}
        limit={100}
        valueText={[m.usage_left({ percent: left }), resetText(w.resets_at)].filter(Boolean).join(' · ')}
      />
    </div>
  {/each}
</div>

<style>
  .windows { display: flex; flex-direction: column; gap: var(--space-3); min-width: 0; }
</style>
