<script lang="ts">
  import type { RequestSummary } from '$lib/api.js';
  import { Badge, Spinner } from '$lib/components/index.js';
  import { formatDateTime } from '$lib/format.js';
  import { formatDuration, requestStatusLabel } from '$lib/status.js';
  import { m } from '$lib/paraglide/messages.js';
  import { Dialog } from 'bits-ui';
  import { XIcon } from 'lucide-svelte';
  import { fmtCost, fmtTokens } from './request-format.js';
  import RequestTimeline from './RequestTimeline.svelte';
  import StopBadge from './StopBadge.svelte';

  interface Props {
    open: boolean;
    loading: boolean;
    error: string;
    detail: RequestSummary | null;
  }

  let { open = $bindable(), loading, error, detail }: Props = $props();
</script>

<Dialog.Root bind:open>
  <Dialog.Portal>
    <Dialog.Overlay class="drawer-overlay" />
    <Dialog.Content class="drawer-content" aria-describedby={undefined}>
      <div class="drawer-header">
        <Dialog.Title class="drawer-title">{m.request_detail_title()}</Dialog.Title>
        <Dialog.Close class="drawer-close" aria-label={m.common_close()}>
          <XIcon size={18} aria-hidden="true" />
        </Dialog.Close>
      </div>

      <div class="drawer-body" aria-live="polite">
        {#if loading}
          <div class="loading"><Spinner size="sm" /> {m.common_loading()}</div>
        {:else if error}
          <p class="error-msg" role="alert">{error}</p>
        {:else if detail}
          {@const d = detail}
          <dl class="info-grid">
            <dt>{m.request_id_label()}</dt><dd class="mono">{d.request_id}</dd>
            <dt>{m.request_model()}</dt><dd>{d.model}</dd>
            <dt>{m.request_api_type()}</dt><dd class="mono">{d.api_type}</dd>
            <dt>{m.request_status()}</dt><dd><Badge status={d.status} label={requestStatusLabel(d.status)} /></dd>
            <dt>{m.request_connection()}</dt><dd class="mono">{d.connection_id ?? m.common_none()}</dd>
            <dt>{m.request_started()}</dt><dd class="mono">{formatDateTime(d.started_at_ms)}</dd>
            <dt>{m.request_duration()}</dt><dd class="mono">{formatDuration(d.duration_ms)}</dd>
            <dt>{m.request_tokens()}</dt><dd class="mono">{fmtTokens(d)}</dd>
            <dt>{m.request_cost()}</dt><dd class="mono">{fmtCost(d.cost_microdollars)}</dd>
          </dl>

          {#if d.stop_reason || d.error_message || d.thinking_requested != null || d.message_count != null || d.cache_read_tokens != null || d.cache_write_tokens != null || d.context_usage_pct != null}
            <h3 class="drawer-subtitle">{m.request_metadata()}</h3>
            <dl class="info-grid">
              {#if d.stop_reason}
                <dt>{m.request_stop_reason()}</dt>
                <dd><StopBadge reason={d.stop_reason} /></dd>
              {/if}
              {#if d.error_message}
                <dt>{m.request_error_message()}</dt>
                <dd class="mono error-text">{d.error_message}</dd>
              {/if}
              {#if d.thinking_requested != null}
                <dt>{m.request_thinking()}</dt>
                <dd>{d.thinking_requested ? 'yes' : 'no'}</dd>
              {/if}
              {#if d.message_count != null}
                <dt>{m.request_message_count()}</dt>
                <dd>{d.message_count}</dd>
              {/if}
              {#if d.cache_read_tokens != null}
                <dt>{m.request_cache_read()}</dt>
                <dd class="mono">{#if d.cache_read_tokens > 0}<span class="cache-badge">{d.cache_read_tokens.toLocaleString()}</span>{:else}–{/if}</dd>
              {/if}
              {#if d.cache_write_tokens != null}
                <dt>{m.request_cache_write()}</dt>
                <dd class="mono">{d.cache_write_tokens.toLocaleString()}</dd>
              {/if}
              {#if d.context_usage_pct != null}
                {@const pct = d.context_usage_pct}
                <dt>Context usage</dt>
                <dd class="mono">
                  <span class="ctx-bar" title="{pct.toFixed(1)}% of context window">
                    <span class="ctx-bar__fill" style="width: {Math.min(pct, 100).toFixed(1)}%"></span>
                  </span>
                  {pct.toFixed(1)}%
                </dd>
              {/if}
            </dl>
          {/if}

          {#if d.state_transitions && d.state_transitions.length > 0}
            <h3 class="drawer-subtitle">Pipeline timeline</h3>
            <RequestTimeline transitions={d.state_transitions} />
          {/if}

          <h3 class="drawer-subtitle">{m.request_decision()}</h3>
          {#if d.decision}
            <dl class="info-grid">
              <dt>{m.request_route()}</dt><dd class="mono">{d.decision.route_id ?? m.common_none()}</dd>
              <dt>{m.request_attempts()}</dt><dd>{d.decision.attempt_count}</dd>
            </dl>
            <h4 class="drawer-subtitle small">{m.request_excluded()}</h4>
            {#if d.decision.candidates_excluded.length === 0}
              <p class="hint">{m.request_excluded_none()}</p>
            {:else}
              <div class="table-wrap drawer-table">
                <table>
                  <thead>
                    <tr>
                      <th scope="col">{m.request_connection()}</th>
                      <th scope="col">{m.request_reason()}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {#each d.decision.candidates_excluded as ex (ex.id)}
                      <tr><td class="mono">{ex.id}</td><td>{ex.reason}</td></tr>
                    {/each}
                  </tbody>
                </table>
              </div>
            {/if}
          {:else}
            <p class="hint">{m.request_no_decision()}</p>
          {/if}
        {/if}
      </div>
    </Dialog.Content>
  </Dialog.Portal>
</Dialog.Root>

<style>
  /* Rendered through a portal, so everything here is scoped to this component's own markup. */
  .loading {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-3);
    font-size: var(--text-sm);
    padding: var(--space-3) 0;
  }

  .drawer-subtitle {
    font-size: var(--text-sm);
    font-weight: var(--weight-semibold);
    color: var(--text-1);
    margin: var(--space-6) 0 var(--space-3);
  }

  .drawer-subtitle.small {
    font-size: var(--text-xs);
    color: var(--text-2);
    margin-top: var(--space-4);
  }

  .info-grid {
    display: grid;
    grid-template-columns: minmax(calc(var(--space-7) * 2), max-content) minmax(0, 1fr);
    gap: 0;
    font-size: var(--text-sm);
    margin: 0;
    background: var(--bg-inset);
    border: var(--border-w) solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
  }

  .info-grid dt,
  .info-grid dd {
    padding: var(--space-2) var(--space-3);
    border-bottom: var(--border-w) solid var(--border);
    display: flex;
    align-items: center;
    flex-wrap: wrap;
  }

  .info-grid dt:nth-last-of-type(1),
  .info-grid dd:nth-last-of-type(1) { border-bottom: 0; }

  dt {
    color: var(--text-3);
    font-weight: var(--weight-medium);
  }

  dd {
    margin: 0;
    color: var(--text-1);
    overflow-wrap: anywhere;
  }

  @media (max-width: 420px) {
    .info-grid { grid-template-columns: minmax(0, 1fr); }
    .info-grid dt { padding-bottom: 0; border-bottom: 0; font-size: var(--text-xs); }
    .info-grid dd { padding-top: var(--space-0); }
  }

  .hint {
    font-size: var(--text-sm);
    color: var(--text-3);
  }

  .drawer-table {
    border: var(--border-w) solid var(--border);
    border-radius: var(--radius);
  }

  .drawer-table table { min-width: 0; }

  .cache-badge {
    display: inline-block;
    font-size: var(--text-2xs);
    font-weight: var(--weight-medium);
    padding: var(--space-0) var(--space-2);
    border-radius: var(--radius-full);
    white-space: nowrap;
    background: var(--success-subtle);
    border: var(--border-w) solid color-mix(in oklch, var(--success) 28%, transparent);
    color: var(--success);
  }

  .ctx-bar {
    display: inline-block;
    vertical-align: middle;
    width: calc(var(--space-8) * 1.5);
    height: var(--meter-h);
    background: var(--border);
    border-radius: var(--radius-full);
    overflow: hidden;
    margin-right: var(--space-2);
  }

  .ctx-bar__fill {
    display: block;
    height: 100%;
    background: var(--accent);
    border-radius: var(--radius-full);
    transition: width var(--dur-3) var(--ease-out);
  }

  .error-text {
    color: var(--danger);
    font-size: var(--text-xs);
    word-break: break-all;
  }
</style>
