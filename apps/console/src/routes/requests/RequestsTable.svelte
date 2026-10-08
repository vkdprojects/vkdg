<script lang="ts">
  import type { RequestSummary } from '$lib/api.js';
  import { Badge } from '$lib/components/index.js';
  import { formatTime } from '$lib/format.js';
  import { formatDuration, requestStatusLabel } from '$lib/status.js';
  import { m } from '$lib/paraglide/messages.js';
  import { fmtCost, fmtTokens } from './request-format.js';
  import StopBadge from './StopBadge.svelte';

  interface Props {
    requests: RequestSummary[];
    onopen: (id: string) => void;
  }

  let { requests, onopen }: Props = $props();
</script>

<div class="table-wrap">
  <table>
    <thead>
      <tr>
        <th scope="col">{m.request_time()}</th>
        <th scope="col">{m.request_model()}</th>
        <th scope="col">{m.request_api_type()}</th>
        <th scope="col">{m.request_status()}</th>
        <th scope="col">{m.request_connection()}</th>
        <th scope="col">{m.request_duration()}</th>
        <th scope="col">{m.request_tokens()}</th>
        <th scope="col">{m.request_cost()}</th>
      </tr>
    </thead>
    <tbody>
      {#each requests as r (r.request_id)}
        <tr class="clickable" onclick={() => onopen(r.request_id)}>
          <td class="mono">
            <!-- The button makes the row reachable by keyboard; the row click is a mouse shortcut. -->
            <button
              type="button"
              class="row-link"
              aria-label={m.request_open_detail({ id: r.request_id })}
              onclick={(e) => { e.stopPropagation(); onopen(r.request_id); }}
            >
              {formatTime(r.started_at_ms)}
            </button>
          </td>
          <td class="model-cell">{r.model}</td>
          <td class="mono">{r.api_type}</td>
          <td>
            <div class="status-cell">
              <Badge status={r.status} label={requestStatusLabel(r.status)} />
              {#if r.stop_reason}
                <StopBadge reason={r.stop_reason} />
              {/if}
            </div>
          </td>
          <td class="mono">{r.connection_id ?? m.common_none()}</td>
          <td class="mono">{formatDuration(r.duration_ms)}</td>
          <td class="mono">{fmtTokens(r)}</td>
          <td class="mono">{fmtCost(r.cost_microdollars)}</td>
        </tr>
      {/each}
    </tbody>
  </table>
</div>

<style>
  table { min-width: calc(var(--space-8) * 11); }
  th { background: var(--bg-inset); }

  .clickable { cursor: pointer; }
  .model-cell { color: var(--text-1); font-weight: var(--weight-medium); white-space: nowrap; }

  .row-link {
    background: none;
    border: none;
    padding: var(--space-1) 0;
    min-height: var(--control-h-sm);
    color: var(--accent);
    font: inherit;
    cursor: pointer;
    white-space: nowrap;
  }

  .row-link:hover { color: var(--accent-hover); text-decoration: underline; }

  .status-cell {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
</style>
