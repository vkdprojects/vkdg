<script lang="ts">
  import { RefreshCwIcon, Trash2Icon, ZapIcon } from 'lucide-svelte';
  import type { Account, ConnectionSummary, ConnectionTestResult } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { Badge, Button, Meter, Spinner, StatusDot } from '$lib/components/index.js';
  import { connectionStatusLabel } from '$lib/status.js';

  interface Props {
    connection: ConnectionSummary;
    account?: Account;
    testing: boolean;
    syncing: boolean;
    /** Latest inline test outcome; cleared by the owner after a few seconds. */
    testResult?: ConnectionTestResult;
    onTest: (conn: ConnectionSummary) => void;
    onSync: (conn: ConnectionSummary) => void;
    onDelete: (conn: ConnectionSummary) => void;
  }

  let { connection: c, account: acct, testing, syncing, testResult: result, onTest, onSync, onDelete }: Props = $props();
</script>

<li class="conn glass" data-status={c.status}>
  <div class="conn-main">
    <div class="conn-title">
      <StatusDot status={c.status} />
      <span class="mono conn-id" title={c.id}>{c.id}</span>
      <Badge status={c.status} label={connectionStatusLabel(c.status)} />
    </div>
    <div class="conn-sub">
      <span class="chip" title={c.models.join('\n')}>{c.model_count === 1 ? m.pd_models_count_one() : m.pd_models_count({ n: c.model_count })}</span>
      {#if acct}<span class="muted conn-acct">{m.pd_account_label({ label: acct.label })}</span>{/if}
    </div>
  </div>

  <div class="conn-load">
    <Meter
      bare
      value={c.active_requests}
      limit={c.max_concurrent}
      ariaLabel={m.pd_in_flight_label({ active: c.active_requests, max: c.max_concurrent })}
    />
    <span class="mono conn-load-text">{c.active_requests} / {c.max_concurrent}</span>
  </div>

  <div class="actions-row conn-actions">
    <span class="test-result" role="status" aria-live="polite">
      {#if result}
        <span class="test-badge" class:ok={result.ok} class:err={!result.ok} title={result.error ?? ''}>
          {result.ok ? m.pd_test_ok({ ms: result.latency_ms }) : m.pd_test_fail()}
        </span>
      {/if}
    </span>
    <Button variant="outline" size="sm" disabled={testing} onclick={() => onTest(c)} ariaLabel={m.pd_test_label({ id: c.id })}>
      {#if testing}<Spinner size="sm" />{:else}<ZapIcon size={14} aria-hidden="true" />{/if}
      {m.connection_test()}
    </Button>
    <span class="secondary-actions actions-row">
      <Button variant="outline" size="sm" disabled={syncing} onclick={() => onSync(c)} ariaLabel={m.pd_sync_label({ id: c.id })}>
        {#if syncing}<Spinner size="sm" />{:else}<RefreshCwIcon size={14} aria-hidden="true" />{/if}
        {m.connection_sync_models()}
      </Button>
      <Button variant="danger" size="sm" onclick={() => onDelete(c)} ariaLabel={m.pd_delete_label({ id: c.id })}>
        <Trash2Icon size={14} aria-hidden="true" /> {m.connection_delete()}
      </Button>
    </span>
  </div>
</li>

<style>
  .conn {
    position: relative;
    display: grid; align-items: center;
    grid-template-columns: minmax(0, 1.4fr) minmax(var(--col-sm), 0.8fr) auto;
    gap: var(--space-3) var(--space-5);
    padding: var(--space-3) var(--space-4) var(--space-3) var(--space-5);
    min-width: 0;
    transition: border-color var(--dur-2) var(--ease-out), transform var(--dur-2) var(--ease-out);
  }
  /* status edge */
  .conn::before {
    content: ''; position: absolute; inset: var(--space-3) auto var(--space-3) 0;
    width: var(--indicator-w); border-radius: var(--radius-full); background: var(--text-3);
  }
  .conn[data-status='healthy']::before { background: var(--success); }
  .conn[data-status='degraded']::before { background: var(--warning); }
  .conn[data-status='circuit_open']::before { background: var(--danger); }
  .conn[data-status='cooldown']::before { background: var(--cooldown); }
  .conn:hover, .conn:focus-within { border-color: var(--accent-strong); }

  .conn-main { display: flex; flex-direction: column; gap: var(--space-2); min-width: 0; }
  .conn-title { display: flex; align-items: center; gap: var(--space-2); min-width: 0; flex-wrap: wrap; }
  .conn-id { color: var(--text-1); font-weight: var(--weight-semibold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; max-width: 100%; }
  .conn-sub { display: flex; align-items: center; gap: var(--space-2); flex-wrap: wrap; font-size: var(--text-xs); min-width: 0; }
  .conn-acct { overflow-wrap: anywhere; }

  .conn-load { display: flex; align-items: center; gap: var(--space-3); min-width: 0; }
  .conn-load :global(.meter) { flex: 1; }
  .conn-load-text { color: var(--text-2); font-size: var(--text-xs); flex-shrink: 0; min-width: 6ch; text-align: right; }

  .conn-actions { justify-content: flex-end; flex-wrap: nowrap; min-width: 0; }
  .test-result { display: inline-flex; align-items: center; min-width: 0; }
  .test-badge {
    display: inline-flex; align-items: center; height: var(--control-h-sm);
    padding: 0 var(--space-3); border-radius: var(--radius-full);
    font-size: var(--text-xs); font-weight: var(--weight-medium); white-space: nowrap;
    animation: pop var(--dur-2) var(--ease-spring);
  }
  .test-badge.ok { background: var(--success-subtle); color: var(--success); }
  .test-badge.err { background: var(--danger-subtle); color: var(--danger); }

  /* Secondary actions: revealed on hover/focus where hover exists; always on for touch. */
  .secondary-actions { flex-wrap: nowrap; transition: opacity var(--dur-2) var(--ease-out); }
  @media (hover: hover) {
    .secondary-actions { opacity: 0; }
    .conn:hover .secondary-actions, .conn:focus-within .secondary-actions { opacity: 1; }
  }

  @keyframes pop { from { opacity: 0; transform: scale(0.9); } }

  /* Narrow: stack rows; keep every action reachable and un-clipped */
  @media (max-width: 800px) {
    .conn { grid-template-columns: minmax(0, 1fr); padding-right: var(--space-4); }
    .conn-actions { justify-content: flex-start; flex-wrap: wrap; }
    .test-result { flex: 1 1 100%; order: 3; }
    .test-result:empty { display: none; }
  }
  @media (prefers-reduced-motion: reduce) {
    .test-badge { animation: none; }
    .conn, .secondary-actions { transition: none; }
  }
</style>
