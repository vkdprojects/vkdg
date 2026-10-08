<script lang="ts">
  import { FlameIcon, RefreshCwIcon } from 'lucide-svelte';
  import type { ConnectionSummary } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { Button } from '$lib/components/index.js';
  import { clock } from '$lib/live.svelte.js';
  import { formatCountdown, secondsUntil } from '$lib/status.js';

  interface Props {
    /** Connections that are not taking traffic right now. */
    connections: ConnectionSummary[];
    resetting: Record<string, boolean>;
    onReset: (conn: ConnectionSummary) => void;
  }

  let { connections, resetting, onReset }: Props = $props();

  // The 1s ticker only runs while this panel is mounted, i.e. while something is cooling.
  const ticker = clock(1000);
</script>

<section class="cooling glass" aria-label={m.pd_cooling_title()}>
  <div class="cooling-head">
    <span class="pulse-dot" aria-hidden="true"></span>
    <div>
      <h2 class="section-title">{m.pd_cooling_title()}</h2>
      <p class="refresh-note">{m.pd_cooling_desc()}</p>
    </div>
  </div>
  <ul class="cooling-list">
    {#each connections as c (c.id)}
      {@const left = c.cooldown_until ? secondsUntil(c.cooldown_until, ticker.now) : null}
      <li class="cooling-row">
        <div class="cooling-id">
          <FlameIcon size={14} aria-hidden="true" />
          <span class="mono" title={c.id}>{c.id}</span>
        </div>
        <div class="cooling-meta">
          {#if left !== null}
            <span class="countdown mono" role="timer" aria-live="off">
              {left > 0 ? m.pd_cooling_remaining({ time: formatCountdown(left) }) : m.pd_cooling_ready()}
            </span>
          {/if}
          {#if c.failure_count != null}<span class="muted">{m.connection_failures({ n: c.failure_count })}</span>{/if}
        </div>
        <Button
          variant="outline"
          size="sm"
          disabled={resetting[c.id]}
          onclick={() => onReset(c)}
          ariaLabel={m.pd_cooling_reset_label({ id: c.id })}
        >
          <RefreshCwIcon size={14} aria-hidden="true" />
          {m.connection_reset_cooldown()}
        </Button>
      </li>
    {/each}
  </ul>
</section>

<style>
  .cooling {
    margin-bottom: var(--space-5); padding: var(--space-4) var(--space-5);
    background: color-mix(in oklch, var(--warning) 9%, var(--glass-bg));
    border-color: color-mix(in oklch, var(--warning) 38%, transparent);
    animation: rise var(--dur-3) var(--ease-out);
  }
  .section-title { font-size: var(--text-lg); font-weight: var(--weight-semibold); color: var(--text-1); margin: 0; }
  .refresh-note { color: var(--text-3); font-size: var(--text-xs); margin: var(--space-0) 0 0; }
  .cooling-head { display: flex; align-items: center; gap: var(--space-3); margin-bottom: var(--space-3); }
  .pulse-dot {
    width: var(--dot-size); height: var(--dot-size); flex-shrink: 0; border-radius: var(--radius-full);
    background: var(--warning);
    animation: warn-pulse var(--dur-pulse) ease-in-out infinite;
  }
  .cooling-list { list-style: none; margin: 0; padding: 0; display: grid; gap: var(--space-2); }
  .cooling-row {
    display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2) var(--space-4);
    padding: var(--space-2) var(--space-3);
    background: var(--bg-inset); border: var(--border-w) solid var(--border); border-radius: var(--radius);
    min-width: 0;
  }
  .cooling-id { display: flex; align-items: center; gap: var(--space-2); color: var(--warning); flex: 1 1 var(--col-sm); min-width: 0; }
  .cooling-id .mono { color: var(--text-1); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .cooling-meta { display: flex; align-items: center; gap: var(--space-3); font-size: var(--text-xs); flex-wrap: wrap; }
  .countdown { color: var(--warning); font-weight: var(--weight-semibold); font-size: var(--text-sm); min-width: 10ch; text-align: right; }

  @keyframes warn-pulse {
    0%, 100% { box-shadow: 0 0 0 0 color-mix(in oklch, var(--warning) 55%, transparent); }
    50% { box-shadow: 0 0 0 var(--space-2) color-mix(in oklch, var(--warning) 0%, transparent); }
  }
  @keyframes rise { from { opacity: 0; transform: translateY(calc(var(--space-2) * -1)); } }

  @media (max-width: 480px) {
    .cooling { padding: var(--space-3) var(--space-4); }
    .countdown { text-align: left; }
  }
  @media (prefers-reduced-motion: reduce) {
    .pulse-dot, .cooling { animation: none; }
  }
</style>
