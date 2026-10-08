<script lang="ts">
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import { ExternalLink, PlusIcon, ArrowLeftIcon } from 'lucide-svelte';
  import { api } from '$lib/api.js';
  import type { Account, ConnectionSummary, ConnectionTestResult, OAuthProvider } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { formatTime } from '$lib/format.js';
  import { deleteConnection, syncConnectionModels } from '$lib/connection-actions.js';
  import { AddConnectionDialog, Button, ConfirmDeleteDialog, EmptyState, RefreshButton, Spinner, Stat, ProviderLogo } from '$lib/components/index.js';
  import { poll } from '$lib/live.svelte.js';
  import { isCooling } from '$lib/status.js';
  import ConnectAccountModal from '../../accounts/ConnectAccountModal.svelte';
  import AccountCard from './AccountCard.svelte';
  import ConnectionRow from './ConnectionRow.svelte';
  import CoolingPanel from './CoolingPanel.svelte';
  import { toast } from 'svelte-sonner';

  const id = $derived(decodeURIComponent($page.params['id'] ?? ''));
  let provider = $state<OAuthProvider | null>(null);
  let accounts = $state<Account[]>([]);
  let connections = $state<ConnectionSummary[]>([]);
  let loading = $state(true);
  let refreshing = $state(false);
  let updatedAt = $state<Date | null>(null);
  let connectOpen = $state(false);
  let connectAccountId = $state<string | null>(null);
  let deleting = $state<string | null>(null);
  let dialogOpen = $state(false);

  // ── Data loading ─────────────────────────────────────────────────────────────
  async function load() {
    refreshing = true;
    try {
      const [pRes, aRes, cRes] = await Promise.all([
        api.oauthProviders(), api.listAccounts(), api.listConnections(),
      ]);
      provider = pRes.items.find((p) => p.id === id) ?? null;
      accounts = aRes.items.filter((a) => a.provider === id);
      connections = cRes.items.filter((c) => c.provider === id);
      updatedAt = new Date();
      if (!provider && !loading) goto('/providers');
    } catch (e) {
      toast.error((e as Error).message);
    } finally {
      loading = false;
      refreshing = false;
    }
  }

  poll(load);

  function connect(accountId?: string) { connectAccountId = accountId ?? null; connectOpen = true; }
  function onConnected(_a: Account) { connectOpen = false; load(); }

  async function deleteAccount(a: Account) {
    if (!confirm(m.acct_delete_confirm({ label: a.label }))) return;
    deleting = a.id;
    try { await api.deleteAccount(a.id); await load(); } finally { deleting = null; }
  }

  const coolingConnections = $derived(connections.filter(isCooling));

  // ── Hero stats ───────────────────────────────────────────────────────────────
  const healthyCount = $derived(connections.filter((c) => c.status === 'healthy').length);
  const activeTotal = $derived(connections.reduce((s, c) => s + c.active_requests, 0));
  const capacityTotal = $derived(connections.reduce((s, c) => s + c.max_concurrent, 0));
  const categoryLabels: Record<OAuthProvider['category'], () => string> = {
    llm_api: m.pd_category_llm_api, oauth_ide: m.pd_category_oauth_ide, compatible: m.pd_category_compatible,
  };

  // ── Connection health filter ─────────────────────────────────────────────────
  type Health = 'all' | 'healthy' | 'issues';
  let health = $state<Health>('all');
  const issueCount = $derived(connections.filter((c) => c.status !== 'healthy').length);
  const visibleConnections = $derived(
    health === 'all' ? connections : connections.filter((c) => (health === 'healthy') === (c.status === 'healthy')),
  );

  // ── Cooldown reset (optimistic, rolls back on error) ─────────────────────────
  let resetting = $state<Record<string, boolean>>({});

  async function resetCooldown(conn: ConnectionSummary) {
    const original = $state.snapshot(conn) as ConnectionSummary;
    const optimistic: ConnectionSummary = { ...original, status: 'healthy', cooldown_until: undefined, failure_count: undefined };
    resetting[conn.id] = true;
    connections = connections.map((c) => (c.id === conn.id ? optimistic : c));
    try {
      await api.resetConnectionCooldown(conn.id);
      toast.success(m.connection_cooldown_reset({ id: conn.id }));
      await load();
    } catch (e) {
      // Roll back only this row so a poll that landed meanwhile is not clobbered.
      connections = connections.map((c) => (c.id === conn.id ? original : c));
      toast.error((e as Error).message);
    } finally {
      delete resetting[conn.id];
    }
  }

  // ── Test: spinner → inline latency result for 4s ─────────────────────────────
  let testing = $state<Record<string, boolean>>({});
  let testResults = $state<Record<string, ConnectionTestResult>>({});
  const testTimers = new Map<string, number>();

  async function testConn(conn: ConnectionSummary) {
    testing[conn.id] = true;
    let result: ConnectionTestResult;
    try {
      result = await api.testConnection(conn.id);
    } catch (e) {
      result = { latency_ms: 0, ok: false, error: (e as Error).message };
    }
    delete testing[conn.id];
    testResults[conn.id] = result;
    window.clearTimeout(testTimers.get(conn.id));
    testTimers.set(conn.id, window.setTimeout(() => { delete testResults[conn.id]; testTimers.delete(conn.id); }, 4000));
  }

  $effect(() => () => { for (const t of testTimers.values()) window.clearTimeout(t); });

  // ── Sync models ──────────────────────────────────────────────────────────────
  let syncing = $state<Record<string, boolean>>({});

  async function syncModels(conn: ConnectionSummary) {
    syncing[conn.id] = true;
    const ok = await syncConnectionModels(conn.id);
    delete syncing[conn.id];
    if (ok) await load();
  }

  // ── Delete connection ────────────────────────────────────────────────────────
  let pendingDelete = $state<ConnectionSummary | null>(null);
  let deletingConn = $state(false);

  async function confirmDeleteConn() {
    if (!pendingDelete) return;
    deletingConn = true;
    const ok = await deleteConnection(pendingDelete.id);
    deletingConn = false;
    if (!ok) return;
    pendingDelete = null;
    await load();
  }

  /** Same upstream user resolved under more than one local account: a real, flaggable conflict. */
  const duplicateUserRefs = $derived.by(() => {
    const seen = new Map<string, string[]>();
    for (const a of accounts) {
      if (!a.credits_user_ref) continue;
      const ids = seen.get(a.credits_user_ref) ?? [];
      ids.push(a.id);
      seen.set(a.credits_user_ref, ids);
    }
    return new Set([...seen.values()].filter((ids) => ids.length > 1).flat());
  });
</script>

<div class="page">
  {#if loading}
    <div class="loading"><Spinner size="sm" /> {m.common_loading()}</div>
  {:else if provider}
    <a class="back" href="/providers"><ArrowLeftIcon size={14} aria-hidden="true" /> {m.pd_back()}</a>

    <!-- Hero: the page's one bold moment -->
    <header class="hero glass">
      <div class="hero-id">
        <ProviderLogo id={provider.id} name={provider.display_name} fallbackChar={provider.icon_char} fallbackColor={provider.icon_color} size="lg" />
        <div class="hero-text">
          <div class="name-row">
            <h1 class="page-title">{provider.display_name}</h1>
            {#if provider.site_url}
              <a class="site-link" href={provider.site_url} target="_blank" rel="noopener noreferrer" aria-label={m.providers_open_site({ name: provider.display_name })}>
                <ExternalLink size={14} aria-hidden="true" />
              </a>
            {/if}
          </div>
          <span class="chip">{(categoryLabels[provider.category] ?? m.pd_category_compatible)()}</span>
          {#if provider.description}<p class="desc">{provider.description}</p>{/if}
        </div>
      </div>
      <div class="hero-stats">
        <Stat
          label={m.pd_stat_connections()}
          value={healthyCount}
          unit={`/ ${connections.length}`}
          tone={connections.length === 0 ? 'default' : healthyCount === connections.length ? 'success' : 'warning'}
        />
        <Stat label={m.pd_stat_in_flight()} value={activeTotal} unit={`/ ${capacityTotal}`} />
        <Stat label={m.pd_stat_accounts()} value={accounts.length} />
      </div>
    </header>

    <!-- Cooling panel: only while something is cooling down -->
    {#if coolingConnections.length > 0}
      <CoolingPanel connections={coolingConnections} {resetting} onReset={resetCooldown} />
    {/if}

    <!-- Accounts -->
    <div class="section-header">
      <div>
        <h2 class="section-title">{m.provider_detail_accounts()}</h2>
        {#if updatedAt}<p class="refresh-note" aria-live="polite">{m.connection_auto_refresh()} {m.common_updated_at({ time: formatTime(updatedAt) })}</p>{/if}
      </div>
      <div class="page-actions">
        <RefreshButton busy={refreshing} onRefresh={load} />
        {#if provider.category === 'oauth_ide'}<Button size="sm" onclick={() => connect()}>{m.provider_detail_connect()}</Button>{/if}
        <Button size="sm" onclick={() => (dialogOpen = true)}><PlusIcon size={14} aria-hidden="true" /> {m.connection_add()}</Button>
      </div>
    </div>

    {#if accounts.length === 0 && provider.category === 'oauth_ide'}
      <EmptyState title={m.provider_detail_no_accounts()} description={m.providers_connect_first()} />
    {:else if accounts.length > 0}
      <div class="accounts-stack">
        {#each accounts as a (a.id)}
          <AccountCard
            account={a}
            connections={connections.filter((c) => c.account_id === a.id)}
            duplicateUser={duplicateUserRefs.has(a.id)}
            deleting={deleting === a.id}
            onConnect={connect}
            onDelete={deleteAccount}
            onChanged={load}
          />
        {/each}
      </div>
    {/if}

    <!-- Connections -->
    <section class="connections" aria-labelledby="pd-connections-title">
      <div class="section-header">
        <h2 class="section-title" id="pd-connections-title">{m.pd_connections_title()}</h2>
        {#if connections.length > 0}
          <div class="segmented" role="group" aria-label={m.pd_filter_label()}>
            <button type="button" aria-pressed={health === 'all'} onclick={() => (health = 'all')}>{m.pd_filter_all()} <span class="count mono">{connections.length}</span></button>
            <button type="button" aria-pressed={health === 'healthy'} onclick={() => (health = 'healthy')}>{m.pd_filter_healthy()} <span class="count mono">{healthyCount}</span></button>
            <button type="button" aria-pressed={health === 'issues'} onclick={() => (health = 'issues')}>{m.pd_filter_issues()} <span class="count mono">{issueCount}</span></button>
          </div>
        {/if}
      </div>

      {#if connections.length === 0}
        <EmptyState title={m.connection_empty()} description={m.connection_empty_desc()} />
      {:else if visibleConnections.length === 0}
        <EmptyState title={m.pd_filter_empty()} />
      {:else}
        <ul class="conn-list">
          {#each visibleConnections as c (c.id)}
            <ConnectionRow
              connection={c}
              account={accounts.find((a) => a.id === c.account_id)}
              testing={!!testing[c.id]}
              syncing={!!syncing[c.id]}
              testResult={testResults[c.id]}
              onTest={testConn}
              onSync={syncModels}
              onDelete={(conn) => (pendingDelete = conn)}
            />
          {/each}
        </ul>
      {/if}
    </section>
  {/if}
</div>

<ConfirmDeleteDialog
  open={pendingDelete !== null}
  onClose={() => (pendingDelete = null)}
  title={m.connection_delete_title()}
  description={m.connection_delete_confirm({ id: pendingDelete?.id ?? '' })}
  confirmLabel={m.connection_delete()}
  busy={deletingConn}
  onConfirm={confirmDeleteConn}
/>

<!-- Connect account modal -->
{#if provider}
  <ConnectAccountModal
    bind:open={connectOpen}
    provider={id}
    accountId={connectAccountId ?? undefined}
    onconnected={onConnected}
  />
{/if}

<AddConnectionDialog bind:open={dialogOpen} initialProvider={id} />

<style>
  .loading { display: flex; align-items: center; gap: var(--space-2); color: var(--text-3); font-size: var(--text-sm); padding: var(--space-5) 0; }
  .back {
    color: var(--text-3); font-size: var(--text-sm); text-decoration: none;
    display: inline-flex; align-items: center; gap: var(--space-1); min-height: var(--control-h-sm);
    margin-bottom: var(--space-3); transition: color var(--dur-1) var(--ease-out), gap var(--dur-2) var(--ease-out);
  }
  .back:hover { color: var(--accent); gap: var(--space-2); }

  /* ── Hero: the page's one bold moment ── */
  .hero {
    display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between;
    gap: var(--space-5) var(--space-6);
    padding: var(--space-5);
    margin-bottom: var(--space-5);
    min-width: 0;
  }
  .hero-id { display: flex; align-items: center; gap: var(--space-4); min-width: 0; flex: 1 1 var(--col-lg); }
  .hero-text { display: flex; flex-direction: column; align-items: flex-start; gap: var(--space-1); min-width: 0; }
  .name-row { display: flex; align-items: center; gap: var(--space-1); flex-wrap: wrap; min-width: 0; }
  .page-title { margin: 0; overflow-wrap: anywhere; }
  .site-link { color: var(--text-3); display: inline-flex; align-items: center; justify-content: center; min-width: var(--control-h-sm); min-height: var(--control-h-sm); border-radius: var(--radius-sm); }
  .site-link:hover { color: var(--accent); background: var(--bg-hover); }
  .desc { color: var(--text-2); font-size: var(--text-sm); margin: var(--space-1) 0 0; max-width: var(--measure); }
  .hero-stats {
    display: flex; flex-wrap: wrap; gap: var(--space-4) var(--space-6);
    flex: 0 1 auto; min-width: 0;
  }

  .section-header { display: flex; align-items: center; justify-content: space-between; gap: var(--space-3); flex-wrap: wrap; margin-bottom: var(--space-4); }
  .section-title { font-size: var(--text-lg); font-weight: var(--weight-semibold); color: var(--text-1); margin: 0; }
  .refresh-note { color: var(--text-3); font-size: var(--text-xs); margin: var(--space-0) 0 0; }

  .accounts-stack { display: grid; gap: var(--space-4); margin-bottom: var(--space-6); }

  .connections { margin-bottom: var(--space-6); }
  .segmented .count { margin-left: var(--space-1); color: var(--text-3); font-size: var(--text-2xs); }
  .segmented > button[aria-pressed="true"] .count { color: var(--accent); }
  .conn-list { list-style: none; margin: 0; padding: 0; display: grid; gap: var(--space-3); }

  @media (max-width: 480px) {
    .hero { padding: var(--space-4); }
  }
  @media (prefers-reduced-motion: reduce) {
    .back { transition: none; }
  }
</style>
