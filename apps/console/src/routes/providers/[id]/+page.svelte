<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import { ExternalLink, PlusIcon, XIcon, RefreshCwIcon, ArrowLeftIcon, FlameIcon, ZapIcon, Trash2Icon } from 'lucide-svelte';
  import { Dialog, AlertDialog } from 'bits-ui';
  import { api } from '$lib/api.js';
  import type { Account, ConnectionStatus, ConnectionSummary, ConnectionTestResult, OAuthProvider } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { formatDateTime, formatRelativeTime, formatTime } from '$lib/format.js';
  import { AccountCredits, Badge, Button, CopyButton, EmptyState, Meter, Select, Spinner, Stat, StatusDot, ProviderLogo } from '$lib/components/index.js';
  import AccountRouting from '../../accounts/AccountRouting.svelte';
  import ConnectAccountModal from '../../accounts/ConnectAccountModal.svelte';
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

  // ── Connection YAML wizard ───────────────────────────────────────────────────
  let dialogOpen = $state(false);
  let connProvider = $state('');
  let connId = $state('');
  let baseUrl = $state('');
  let envVar = $state('');
  let models = $state('');
  let step = $state<'form' | 'yaml'>('form');

  const providerOptions = [
    { value: 'openai-compat', label: 'OpenAI-compatible (Ollama, vLLM, etc.)' },
    { value: 'anthropic-compat', label: 'Anthropic-compatible' },
    { value: 'anthropic',  label: 'Anthropic (Claude)' },
    { value: 'openai',     label: 'OpenAI (GPT / o-series)' },
    { value: 'groq',       label: 'Groq' },
    { value: 'gemini',     label: 'Google Gemini' },
    { value: 'deepseek',   label: 'DeepSeek' },
    { value: 'mistral',    label: 'Mistral AI' },
    { value: 'together',   label: 'Together AI' },
    { value: 'fireworks',  label: 'Fireworks AI' },
    { value: 'sambanova',  label: 'SambaNova (free tier)' },
    { value: 'cerebras',   label: 'Cerebras (free tier)' },
    { value: 'nvidia-nim', label: 'NVIDIA NIM' },
    { value: 'kiro',       label: 'Kiro (Amazon Q)' },
  ];

  const defaultEnvVar: Record<string, string> = {
    anthropic: 'ANTHROPIC_API_KEY', openai: 'OPENAI_API_KEY', groq: 'GROQ_API_KEY',
    gemini: 'GEMINI_API_KEY', deepseek: 'DEEPSEEK_API_KEY', mistral: 'MISTRAL_API_KEY',
    together: 'TOGETHER_API_KEY', fireworks: 'FIREWORKS_API_KEY', sambanova: 'SAMBANOVA_API_KEY',
    cerebras: 'CEREBRAS_API_KEY', 'nvidia-nim': 'NVIDIA_API_KEY', kiro: 'KIRO_API_KEY',
    'openai-compat': 'API_KEY',
  };
  const defaultModels: Record<string, string> = {
    anthropic: 'claude-*', openai: 'gpt-*, o1-*, o3-*', groq: 'llama-*, mixtral-*',
    gemini: 'gemini-*', deepseek: 'deepseek-*', mistral: 'mistral-*',
    together: 'meta-llama/*', fireworks: 'accounts/*', sambanova: 'Meta-Llama-*',
    cerebras: 'llama3.1-*', 'nvidia-nim': 'meta/llama-*',
    kiro: 'claude-*, gpt-5.6-*, minimax-*, deepseek-*, glm-*, qwen3-*, auto',
  };
  const showBaseUrl = $derived(connProvider === 'openai-compat' || connProvider === 'anthropic-compat');

  $effect(() => {
    if (connProvider in defaultEnvVar) envVar = defaultEnvVar[connProvider];
    if (connProvider in defaultModels) models = defaultModels[connProvider];
    if (!connId) connId = `${connProvider}-default`;
  });

  const yamlSnippet = $derived(() => {
    const modelList = (models || '*').split(',').map((s) => `"${s.trim()}"`).join(', ');
    const lines = [
      'connections:',
      `  - id: ${connId || connProvider + '-default'}`,
      `    provider: ${connProvider}`,
      ...(showBaseUrl && baseUrl ? [`    base_url: ${baseUrl}`] : []),
      '    auth:', '      type: api_key',
      `      env_var: ${envVar || 'API_KEY'}`,
      `    models: [${modelList}]`,
      '    max_concurrent: 50', '    weight: 1',
    ];
    return lines.join('\n');
  });

  function openConnDialog() {
    connProvider = id;
    step = 'form';
    connId = '';
    baseUrl = '';
    dialogOpen = true;
  }

  // ── Status labels ────────────────────────────────────────────────────────────
  const statusLabels: Record<ConnectionStatus, () => string> = {
    healthy: m.connection_status_healthy, degraded: m.connection_status_degraded,
    circuit_open: m.connection_status_circuit_open, cooldown: m.connection_status_cooldown,
    unknown: m.connection_status_unknown,
  };

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

  onMount(() => {
    let timer: ReturnType<typeof setInterval>;
    const sync = () => {
      clearInterval(timer);
      if (document.visibilityState !== 'visible') return;
      load();
      timer = setInterval(load, 5000);
    };
    sync();
    document.addEventListener('visibilitychange', sync);
    return () => { clearInterval(timer); document.removeEventListener('visibilitychange', sync); };
  });

  function connect(accountId?: string) { connectAccountId = accountId ?? null; connectOpen = true; }
  function onConnected(_a: Account) { connectOpen = false; load(); }

  async function deleteAccount(a: Account) {
    if (!confirm(m.acct_delete_confirm({ label: a.label }))) return;
    deleting = a.id;
    try { await api.deleteAccount(a.id); await load(); } finally { deleting = null; }
  }

  // ── Live clocks: client-side timers, not polling ─────────────────────────────
  let now = $state(Date.now());
  let now30 = $state(Date.now());

  /** Seconds left until an RFC 3339 instant; null when unparsable. */
  function secondsUntil(iso: string): number | null {
    const t = new Date(iso).getTime();
    return Number.isFinite(t) ? Math.max(0, Math.ceil((t - now) / 1000)) : null;
  }

  function clock(seconds: number): string {
    const h = Math.floor(seconds / 3600);
    const mm = Math.floor((seconds % 3600) / 60);
    const ss = seconds % 60;
    const pad = (n: number) => String(n).padStart(2, '0');
    return h > 0 ? `${h}:${pad(mm)}:${pad(ss)}` : `${mm}:${pad(ss)}`;
  }

  /** Coarse "in 5 minutes" / "2 hours ago"; refreshed every 30s via `now30`. */
  function relativeFromNow(iso: string): string {
    const t = new Date(iso).getTime();
    if (!Number.isFinite(t)) return m.common_none();
    const seconds = Math.round((t - now30) / 1000);
    const abs = Math.abs(seconds);
    if (abs < 60) return formatRelativeTime(seconds, 'second');
    const minutes = Math.round(seconds / 60);
    if (Math.abs(minutes) < 60) return formatRelativeTime(minutes, 'minute');
    const hours = Math.round(minutes / 60);
    if (Math.abs(hours) < 24) return formatRelativeTime(hours, 'hour');
    return formatRelativeTime(Math.round(hours / 24), 'day');
  }

  function tokenExpired(a: Account): boolean {
    return !!a.expires_at && new Date(a.expires_at).getTime() <= now30;
  }

  const isCooling = (c: ConnectionSummary) => c.status === 'cooldown' || c.status === 'circuit_open' || !!c.cooldown_until;
  const coolingConnections = $derived(connections.filter(isCooling));

  $effect(() => {
    const t = setInterval(() => (now30 = Date.now()), 30_000);
    return () => clearInterval(t);
  });

  // The 1s ticker only runs while something is actually counting down.
  $effect(() => {
    if (coolingConnections.length === 0) return;
    now = Date.now();
    const t = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(t);
  });

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
  const testTimers = new Map<string, ReturnType<typeof setTimeout>>();

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
    clearTimeout(testTimers.get(conn.id));
    testTimers.set(conn.id, setTimeout(() => { delete testResults[conn.id]; testTimers.delete(conn.id); }, 4000));
  }

  // ── Sync models ──────────────────────────────────────────────────────────────
  let syncing = $state<Record<string, boolean>>({});

  async function syncModels(conn: ConnectionSummary) {
    syncing[conn.id] = true;
    try {
      const result = await api.syncConnectionModels(conn.id);
      toast.success(m.connection_models_synced({ count: result.count }));
      await load();
    } catch (e) {
      toast.error((e as Error).message);
    } finally {
      delete syncing[conn.id];
    }
  }

  // ── Delete connection ────────────────────────────────────────────────────────
  let pendingDelete = $state<ConnectionSummary | null>(null);
  let deletingConn = $state(false);

  async function confirmDeleteConn() {
    if (!pendingDelete) return;
    deletingConn = true;
    try {
      await api.deleteConnection(pendingDelete.id);
      toast.success(m.connection_deleted());
      pendingDelete = null;
      await load();
    } catch (e) {
      toast.error((e as Error).message);
    } finally {
      deletingConn = false;
    }
  }

  $effect(() => () => { for (const t of testTimers.values()) clearTimeout(t); });

  function connectionsForAccount(account: Account): ConnectionSummary[] {
    return connections.filter((c) => c.account_id === account.id);
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
      <section class="cooling glass" aria-label={m.pd_cooling_title()}>
        <div class="cooling-head">
          <span class="pulse-dot" aria-hidden="true"></span>
          <div>
            <h2 class="section-title">{m.pd_cooling_title()}</h2>
            <p class="refresh-note">{m.pd_cooling_desc()}</p>
          </div>
        </div>
        <ul class="cooling-list">
          {#each coolingConnections as c (c.id)}
            {@const left = c.cooldown_until ? secondsUntil(c.cooldown_until) : null}
            <li class="cooling-row">
              <div class="cooling-id">
                <FlameIcon size={14} aria-hidden="true" />
                <span class="mono" title={c.id}>{c.id}</span>
              </div>
              <div class="cooling-meta">
                {#if left !== null}
                  <span class="countdown mono" role="timer" aria-live="off">
                    {left > 0 ? m.pd_cooling_remaining({ time: clock(left) }) : m.pd_cooling_ready()}
                  </span>
                {/if}
                {#if c.failure_count != null}<span class="muted">{m.connection_failures({ n: c.failure_count })}</span>{/if}
              </div>
              <Button
                variant="outline"
                size="sm"
                disabled={resetting[c.id]}
                onclick={() => resetCooldown(c)}
                ariaLabel={m.pd_cooling_reset_label({ id: c.id })}
              >
                <RefreshCwIcon size={14} aria-hidden="true" />
                {m.connection_reset_cooldown()}
              </Button>
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    <!-- Accounts -->
    <div class="section-header">
      <div>
        <h2 class="section-title">{m.provider_detail_accounts()}</h2>
        {#if updatedAt}<p class="refresh-note" aria-live="polite">{m.connection_auto_refresh()} {m.common_updated_at({ time: formatTime(updatedAt) })}</p>{/if}
      </div>
      <div class="page-actions">
        <Button variant="outline" size="sm" onclick={load} disabled={refreshing} ariaLabel={m.common_refresh()}>
          <RefreshCwIcon size={14} aria-hidden="true" /> {m.common_refresh()}
        </Button>
        {#if provider.category === 'oauth_ide'}<Button size="sm" onclick={() => connect()}>{m.provider_detail_connect()}</Button>{/if}
        <Button size="sm" onclick={openConnDialog}><PlusIcon size={14} aria-hidden="true" /> {m.connection_add()}</Button>
      </div>
    </div>

    {#if accounts.length === 0 && provider.category === 'oauth_ide'}
      <EmptyState title={m.provider_detail_no_accounts()} description={m.providers_connect_first()} />
    {:else if accounts.length > 0}
      <div class="accounts-stack">
        {#each accounts as a (a.id)}
          {@const expired = tokenExpired(a)}
          <article class="account glass">
            <header class="account-head">
              <div class="account-title">
                <span class="eyebrow">{m.provider_account()}</span>
                <h3>{a.label}</h3>
                <span class="mono account-id">{a.id}</span>
              </div>
              <Badge status={a.status === 'active' ? 'healthy' : 'degraded'} label={a.status === 'active' ? m.acct_status_active() : m.acct_status_needs_login()} />
            </header>

            <div class="account-chips">
              {#if a.expires_at}
                <span class="chip" class:chip-danger={expired} title={formatDateTime(a.expires_at)}>
                  {expired ? m.pd_token_expired() : m.pd_token_expires_in({ when: relativeFromNow(a.expires_at) })}
                </span>
              {:else}
                <span class="chip">{m.pd_token_never()}</span>
              {/if}
              <span class="chip" class:accent={a.has_refresh_token}>{a.has_refresh_token ? m.pd_refresh_token_yes() : m.pd_refresh_token_no()}</span>
              {#if a.revoked_reason}<span class="chip chip-danger" title={m.account_revocation_reason()}>{a.revoked_reason}</span>{/if}
              {#if a.credits_user_ref && duplicateUserRefs.has(a.id)}
                <span class="dup-warning" title={m.account_duplicate_user_desc()}><Badge status="degraded" label={m.account_duplicate_user()} /></span>
              {/if}
            </div>

            <AccountCredits account={a} />

            <div class="connections-block"><AccountRouting account={a} connections={connectionsForAccount(a)} onchanged={load} /></div>

            <footer class="actions-row account-actions">
              <Button
                variant={a.status === 'needs_login' ? 'primary' : 'outline'}
                size="sm"
                onclick={() => connect(a.id)}
                ariaLabel={m.pd_account_reconnect_label({ label: a.label })}
              >
                <RefreshCwIcon size={14} aria-hidden="true" /> {m.acct_reauth()}
              </Button>
              <Button
                variant="danger"
                size="sm"
                disabled={deleting === a.id}
                onclick={() => deleteAccount(a)}
                ariaLabel={m.pd_account_delete_label({ label: a.label })}
              >
                <Trash2Icon size={14} aria-hidden="true" /> {m.acct_delete()}
              </Button>
            </footer>
          </article>
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
            {@const result = testResults[c.id]}
            {@const acct = accounts.find((a) => a.id === c.account_id)}
            <li class="conn glass" data-status={c.status}>
              <div class="conn-main">
                <div class="conn-title">
                  <StatusDot status={c.status} />
                  <span class="mono conn-id" title={c.id}>{c.id}</span>
                  <Badge status={c.status} label={(statusLabels[c.status] ?? m.connection_status_unknown)()} />
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
                <Button variant="outline" size="sm" disabled={testing[c.id]} onclick={() => testConn(c)} ariaLabel={m.pd_test_label({ id: c.id })}>
                  {#if testing[c.id]}<Spinner size="sm" />{:else}<ZapIcon size={14} aria-hidden="true" />{/if}
                  {m.connection_test()}
                </Button>
                <span class="secondary-actions actions-row">
                  <Button variant="outline" size="sm" disabled={syncing[c.id]} onclick={() => syncModels(c)} ariaLabel={m.pd_sync_label({ id: c.id })}>
                    {#if syncing[c.id]}<Spinner size="sm" />{:else}<RefreshCwIcon size={14} aria-hidden="true" />{/if}
                    {m.connection_sync_models()}
                  </Button>
                  <Button variant="danger" size="sm" onclick={() => (pendingDelete = c)} ariaLabel={m.pd_delete_label({ id: c.id })}>
                    <Trash2Icon size={14} aria-hidden="true" /> {m.connection_delete()}
                  </Button>
                </span>
              </div>
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  {/if}
</div>

<AlertDialog.Root open={pendingDelete !== null} onOpenChange={(v) => { if (!v) pendingDelete = null; }}>
  <AlertDialog.Portal>
    <AlertDialog.Overlay class="dialog-overlay" />
    <AlertDialog.Content class="dialog-content">
      <div class="dialog-header">
        <AlertDialog.Title class="dialog-title">{m.connection_delete_title()}</AlertDialog.Title>
      </div>
      <AlertDialog.Description class="dialog-body confirm-desc">
        {m.connection_delete_confirm({ id: pendingDelete?.id ?? '' })}
      </AlertDialog.Description>
      <div class="dialog-footer">
        <AlertDialog.Cancel class="btn-like outline">{m.common_cancel()}</AlertDialog.Cancel>
        <button type="button" class="btn-like danger" disabled={deletingConn} onclick={confirmDeleteConn}>{m.connection_delete()}</button>
      </div>
    </AlertDialog.Content>
  </AlertDialog.Portal>
</AlertDialog.Root>

<!-- Connect account modal -->
{#if provider}
  <ConnectAccountModal
    bind:open={connectOpen}
    provider={id}
    accountId={connectAccountId ?? undefined}
    onconnected={onConnected}
  />
{/if}

<!-- Add connection wizard -->
<Dialog.Root bind:open={dialogOpen} onOpenChange={(v) => { if (!v) step = 'form'; }}>
  <Dialog.Portal>
    <Dialog.Overlay class="dialog-overlay" />
    <Dialog.Content class="dialog-content" aria-describedby={undefined}>
      <div class="dialog-header">
        <Dialog.Title class="dialog-title">{step === 'form' ? m.connection_add() : 'Add to your config'}</Dialog.Title>
        <button class="dialog-close" aria-label={m.common_cancel()} onclick={() => (dialogOpen = false)}>
          <XIcon size={16} aria-hidden="true" />
        </button>
      </div>

      {#if step === 'form'}
        <form onsubmit={(e) => { e.preventDefault(); step = 'yaml'; }}>
          <div class="dialog-body fields">
            <Select label={m.connection_provider()} options={providerOptions} bind:value={connProvider} />
            <div class="field">
              <label for="conn-id">Connection ID</label>
              <input id="conn-id" type="text" bind:value={connId} placeholder="{connProvider}-default" required />
            </div>
            {#if showBaseUrl}
              <div class="field">
                <label for="conn-base-url">Base URL</label>
                <input id="conn-base-url" type="text" bind:value={baseUrl} placeholder="http://localhost:11434" />
              </div>
            {/if}
            <div class="field">
              <label for="conn-env">API key env var</label>
              <input id="conn-env" type="text" bind:value={envVar} placeholder="MY_API_KEY" />
              <span class="field-hint">The key stays in your environment, not in the config.</span>
            </div>
            <div class="field">
              <label for="conn-models">Models (comma-separated globs)</label>
              <input id="conn-models" type="text" bind:value={models} placeholder="claude-*, gpt-4*" />
            </div>
          </div>
          <div class="dialog-footer">
            <Button variant="outline" type="button" onclick={() => (dialogOpen = false)}>{m.common_cancel()}</Button>
            <Button variant="primary" type="submit">Generate config snippet →</Button>
          </div>
        </form>
      {:else}
        <div class="yaml-step">
          <p class="yaml-note">Copy this into your <code>vkdg.yaml</code>. Hot-reload picks it up automatically.</p>
          <div class="yaml-block">
            <pre class="yaml-code">{yamlSnippet()}</pre>
            <CopyButton text={yamlSnippet()} />
          </div>
          <p class="yaml-env-note">
            Set the env var: <code>export {envVar || 'API_KEY'}=your-key-here</code>
          </p>
        </div>
        <div class="dialog-footer">
          <Button variant="outline" onclick={() => (step = 'form')}>← Back</Button>
          <Button variant="primary" onclick={() => (dialogOpen = false)}>Done</Button>
        </div>
      {/if}
    </Dialog.Content>
  </Dialog.Portal>
</Dialog.Root>

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

  /* ── Cooling panel ── */
  .cooling {
    margin-bottom: var(--space-5); padding: var(--space-4) var(--space-5);
    background: color-mix(in oklch, var(--warning) 9%, var(--glass-bg));
    border-color: color-mix(in oklch, var(--warning) 38%, transparent);
    animation: rise var(--dur-3) var(--ease-out);
  }
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

  /* ── Accounts ── */
  .accounts-stack { display: grid; gap: var(--space-4); margin-bottom: var(--space-6); }
  .account { display: flex; flex-direction: column; gap: var(--space-4); padding: var(--space-5); min-width: 0; }
  .account-head { display: flex; align-items: flex-start; justify-content: space-between; gap: var(--space-3); flex-wrap: wrap; }
  .account-title { min-width: 0; }
  .account-title h3 { margin: var(--space-0) 0; font-size: var(--text-md); overflow-wrap: anywhere; }
  .eyebrow { color: var(--text-3); font-size: var(--text-xs); font-weight: var(--weight-medium); }
  .account-id { color: var(--text-3); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .account-chips { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2); }
  .chip-danger { color: var(--danger); background: var(--danger-subtle); border-color: color-mix(in oklch, var(--danger) 35%, transparent); }
  .dup-warning { display: inline-flex; align-items: center; }
  .connections-block { min-width: 0; }
  .account-actions { justify-content: flex-end; padding-top: var(--space-3); border-top: var(--border-w) solid var(--border); }

  /* ── Connections ── */
  .connections { margin-bottom: var(--space-6); }
  .segmented .count { margin-left: var(--space-1); color: var(--text-3); font-size: var(--text-2xs); }
  .segmented > button[aria-pressed="true"] .count { color: var(--accent); }
  .conn-list { list-style: none; margin: 0; padding: 0; display: grid; gap: var(--space-3); }
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

  /* Delete-confirm buttons share the Button look without a second component */
  :global(.btn-like) {
    display: inline-flex; align-items: center; justify-content: center;
    height: var(--control-h); padding: 0 var(--control-px);
    border: var(--border-w) solid var(--border-strong); border-radius: var(--radius);
    background: var(--bg-surface); color: var(--text-1); font: inherit; font-size: var(--text-sm); font-weight: var(--weight-medium);
    cursor: pointer;
  }
  :global(.btn-like:hover) { background: var(--bg-hover); }
  :global(.btn-like.danger) { background: var(--danger); border-color: var(--danger); color: var(--on-accent); }
  :global(.btn-like.danger:hover) { background: color-mix(in oklch, var(--danger) 88%, var(--text-1)); }
  :global(.btn-like:disabled) { opacity: 0.45; cursor: not-allowed; }
  :global(.confirm-desc) { color: var(--text-2); font-size: var(--text-sm); margin: 0; }

  @keyframes warn-pulse {
    0%, 100% { box-shadow: 0 0 0 0 color-mix(in oklch, var(--warning) 55%, transparent); }
    50% { box-shadow: 0 0 0 var(--space-2) color-mix(in oklch, var(--warning) 0%, transparent); }
  }
  @keyframes rise { from { opacity: 0; transform: translateY(calc(var(--space-2) * -1)); } }
  @keyframes pop { from { opacity: 0; transform: scale(0.9); } }

  /* Narrow: stack rows; keep every action reachable and un-clipped */
  @media (max-width: 800px) {
    .conn { grid-template-columns: minmax(0, 1fr); padding-right: var(--space-4); }
    .conn-actions { justify-content: flex-start; flex-wrap: wrap; }
    .test-result { flex: 1 1 100%; order: 3; }
    .test-result:empty { display: none; }
  }
  @media (max-width: 480px) {
    .hero { padding: var(--space-4); }
    .account { padding: var(--space-4); }
    .account-actions > :global(*) { flex: 1 1 auto; }
    .cooling { padding: var(--space-3) var(--space-4); }
    .countdown { text-align: left; }
  }

  @media (prefers-reduced-motion: reduce) {
    .pulse-dot, .test-badge, .cooling { animation: none; }
    .back, .conn, .secondary-actions { transition: none; }
  }

  .yaml-step { display: flex; flex-direction: column; gap: var(--space-3); padding: var(--space-4) var(--space-5); }
  .yaml-note { font-size: var(--text-sm); color: var(--text-2); margin: 0; }
  .yaml-block { position: relative; background: var(--bg-inset); border: var(--border-w) solid var(--border); border-radius: var(--radius-sm); padding: var(--space-3); }
  .yaml-code { font-family: var(--font-mono); font-size: var(--text-xs); line-height: var(--leading); margin: 0; white-space: pre; overflow-x: auto; color: var(--text-1); }
  .yaml-env-note { font-size: var(--text-sm); color: var(--text-3); margin: 0; }
  .yaml-env-note code { font-family: var(--font-mono); }
</style>
