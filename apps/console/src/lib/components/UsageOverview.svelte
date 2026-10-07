<script lang="ts">
  import type { Account } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { formatRelativeTime } from '$lib/format.js';
  import { remainingPercent, resetDelta, usageColumns, usageRows } from '$lib/usage.js';
  import { usageLabels } from '$lib/usage-labels.js';
  import Meter from './Meter.svelte';
  import Badge from './Badge.svelte';
  import Card from './Card.svelte';

  /**
   * Every metered account side by side — one row per account, one bar per
   * window — so the account closest to its limit is visible without opening
   * each card. Rows are sorted most-consumed first.
   */
  interface Props {
    accounts: Account[];
  }

  let { accounts }: Props = $props();

  const rows = $derived(usageRows(accounts));
  const columns = $derived(usageColumns(rows));

  function resetTitle(resetsAt: number | null): string | undefined {
    const delta = resetDelta(resetsAt, Date.now());
    return delta ? m.usage_resets({ when: formatRelativeTime(delta.n, delta.unit) }) : undefined;
  }
</script>

{#if rows.length > 0}
  <Card padding="0">
    <section class="overview" data-testid="usage-overview">
      <header class="head">
        <h2>{m.usage_overview_title()}</h2>
        <p>{m.usage_overview_desc()}</p>
      </header>
      <div class="grid" style="--cols: {columns.length}">
        <div class="cell head-cell">{m.usage_overview_account()}</div>
        {#each columns as col (col)}<div class="cell head-cell">{usageLabels[col]()}</div>{/each}
        {#each rows as row (row.account.id)}
          <div class="cell who" data-account={row.account.id}>
            <span class="provider mono">{row.account.provider}</span>
            <span class="label">{row.account.label}</span>
          </div>
          {#if row.state === 'unavailable' && row.peak === null}
            <div class="cell span"><Badge status="unknown" label={m.usage_overview_unavailable()} /></div>
          {:else}
            {#each columns as col (col)}
              {@const cell = row.cells[col]}
              <div class="cell" title={cell ? resetTitle(cell.resetsAt) : undefined} data-window={col}>
                {#if cell}<Meter value={cell.percent} limit={100} valueText={m.usage_left({ percent: Math.round(remainingPercent(cell.percent)) })} />{:else}<span class="none">—</span>{/if}
              </div>
            {/each}
          {/if}
        {/each}
      </div>
    </section>
  </Card>
{/if}

<style>
  .overview { padding: 1rem; display: flex; flex-direction: column; gap: 0.75rem; }
  .head h2 { margin: 0; font-size: 0.9375rem; color: var(--text-1); }
  .head p { margin: 2px 0 0; font-size: var(--text-xs); color: var(--text-2); }
  .grid {
    display: grid;
    grid-template-columns: minmax(10rem, 1.2fr) repeat(var(--cols), minmax(8rem, 1fr));
    column-gap: 16px;
    row-gap: 10px;
    align-items: center;
    overflow-x: auto;
  }
  .head-cell { color: var(--text-3); font-size: var(--text-2xs); text-transform: uppercase; letter-spacing: 0.06em; }
  .who { display: flex; flex-direction: column; min-width: 0; }
  .provider { color: var(--text-3); font-size: var(--text-2xs); text-transform: uppercase; letter-spacing: 0.06em; }
  .label { color: var(--text-1); font-size: var(--text-sm); overflow-wrap: anywhere; }
  .span { grid-column: 2 / -1; }
  .none { color: var(--text-3); }
</style>
