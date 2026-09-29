<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import { ExternalLink, PlusIcon, XIcon, RefreshCwIcon } from 'lucide-svelte';
  import { Dialog } from 'bits-ui';
  import { api } from '$lib/api.js';
  import type { Account, ConnectionStatus, ConnectionSummary, ConnectionTestResult, OAuthProvider } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { Badge, Button, CopyButton, EmptyState, Meter, Select, Spinner, StatusDot } from '$lib/components/index.js';
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

  // ── Connection test ──────────────────────────────────────────────────────────
  let testing = $state<string | null>(null);
  let testResults = $state<Record<string, ConnectionTestResult>>({});

  async function testConn(connId: string) {
    testing = connId;
    try {
      testResults[connId] = await api.testConnection(connId);
    } catch (e) {
      testResults[connId] = { latency_ms: 0, ok: false, error: (e as Error).message };
    } finally {
      testing = null;
    }
  }

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
  const timeFmt = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit' });

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

  function formatDate(iso: string) {
    return new Intl.DateTimeFormat(undefined, { year: 'numeric', month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }).format(new Date(iso));
  }

  function cooldownRemaining(iso: string): string {
    const seconds = Math.max(0, Math.round((new Date(iso).getTime() - Date.now()) / 1000));
    return seconds >= 60 ? `${Math.floor(seconds / 60)}m ${seconds % 60}s` : `${seconds}s`;
  }

  function connectionsForAccount(account: Account): ConnectionSummary[] {
    return connections.filter((c) => c.account_id === account.id);
  }

  const unassignedConnections = $derived(connections.filter((c) => !c.account_id));

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

  const dateTimeFmt = new Intl.DateTimeFormat(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
  function formatPeriodEnd(unixSecs: number): string {
    return dateTimeFmt.format(new Date(unixSecs * 1000));
  }

  /** Coarse "checked X ago" — a cache-age hint, not a live clock. */
  function checkedAgo(iso: string): string {
    const seconds = Math.max(0, Math.round((Date.now() - new Date(iso).getTime()) / 1000));
    if (seconds < 60) return m.credits_checked_seconds({ n: seconds });
    const minutes = Math.round(seconds / 60);
    if (minutes < 60) return m.credits_checked_minutes({ n: minutes });
    const hours = Math.round(minutes / 60);
    if (hours < 24) return m.credits_checked_hours({ n: hours });
    return m.credits_checked_days({ n: Math.round(hours / 24) });
  }
</script>

<div class="page">
  {#if loading}
    <div class="loading"><Spinner size="sm" /> {m.common_loading()}</div>
  {:else if provider}
    <!-- Header -->
    <div class="header">
      <a class="back" href="/providers">← {m.providers_title()}</a>
      <div class="provider-head">
        <span class="icon" style:background={provider.icon_color}>{provider.icon_char}</span>
        <div>
          <div class="name-row">
            <h1 class="page-title">{provider.display_name}</h1>
            {#if provider.site_url}
              <a class="site-link" href={provider.site_url} target="_blank" rel="noopener noreferrer">
                <ExternalLink size={14} aria-hidden="true" />
              </a>
            {/if}
          </div>
          {#if provider.description}<p class="desc">{provider.description}</p>{/if}
        </div>
      </div>
    </div>

    <div class="section-header">
      <div><h2 class="section-title">{m.provider_detail_accounts()}</h2>{#if updatedAt}<p class="refresh-note">{m.connection_auto_refresh()} {m.common_updated_at({ time: timeFmt.format(updatedAt) })}</p>{/if}</div>
      <div class="header-actions"><Button variant="outline" size="sm" onclick={load} disabled={refreshing} ariaLabel={m.common_refresh()}><RefreshCwIcon size={14} /> {m.common_refresh()}</Button>{#if provider.category === 'oauth_ide'}<Button size="sm" onclick={() => connect()}>{m.provider_detail_connect()}</Button>{/if}<Button size="sm" onclick={openConnDialog}><PlusIcon size={14} /> {m.connection_add()}</Button></div>
    </div>
    {#if accounts.length === 0 && provider.category === 'oauth_ide'}
      <EmptyState title={m.provider_detail_no_accounts()} description={m.providers_connect_first()} />
    {:else}
      <div class="accounts-stack">
        {#each accounts as a (a.id)}
          <article class="account-panel">
            <header class="account-head"><div><span class="eyebrow">{m.provider_account()}</span><h3>{a.label}</h3><span class="mono account-id">{a.id}</span>{#if a.credits_user_ref && duplicateUserRefs.has(a.id)}<div class="dup-warning"><Badge status="degraded" label={m.account_duplicate_user()} /> <span>{m.account_duplicate_user_desc()}</span></div>{/if}</div><div class="account-actions"><Badge status={a.status === 'active' ? 'healthy' : 'degraded'} label={a.status === 'active' ? m.acct_status_active() : m.acct_status_needs_login()} /><Button variant={a.status === 'needs_login' ? 'primary' : 'outline'} size="sm" onclick={() => connect(a.id)}>{m.acct_reauth()}</Button><Button variant="danger" size="sm" disabled={deleting === a.id} onclick={() => deleteAccount(a)}>{m.acct_delete()}</Button></div></header>
            <dl class="account-facts"><div><dt>{m.acct_expires()}</dt><dd>{a.expires_at ? formatDate(a.expires_at) : m.acct_never()}</dd></div><div><dt>{m.acct_refresh_token()}</dt><dd>{a.has_refresh_token ? m.common_yes() : m.common_no()}</dd></div><div><dt>{m.account_revocation_reason()}</dt><dd>{a.revoked_reason ?? m.common_none()}</dd></div></dl>
            {#if a.credits_source === 'reported' && a.credits_used != null && a.credits_limit != null}
              <div class="limits" data-state="reported">
                <div class="limits-head"><span>{m.plan_limits_title()}</span>{#if a.credits_plan}<span class="plan-name">{a.credits_plan}</span>{/if}</div>
                <Meter value={a.credits_used} limit={a.credits_limit} valueText={`${a.credits_used.toLocaleString()} / ${a.credits_limit.toLocaleString()}`} />
                <div class="limits-foot">
                  {#if a.credits_period_end != null}<span>{m.credits_resets_on({ date: formatPeriodEnd(a.credits_period_end) })}</span>{/if}
                  {#if a.credits_checked_at}<span class="checked-at">{m.credits_last_checked({ time: checkedAgo(a.credits_checked_at) })}</span>{/if}
                </div>
              </div>
            {:else if a.credits_source === 'unavailable'}
              <div class="limits" data-state="unavailable"><div class="limits-head"><span>{m.plan_limits_title()}</span><Badge status="unknown" label={m.plan_limits_unavailable()} /></div><p>{m.plan_limits_unavailable_desc()}</p></div>
            {/if}
            <div class="connections-block"><h4>{m.account_connections()}</h4>
              {#if connectionsForAccount(a).length === 0}<p class="muted-note">{m.account_connections_unassigned()}</p>{:else}
                <div class="table-wrap"><table><thead><tr><th>{m.connection_id()}</th><th>{m.connection_status()}</th><th>{m.connection_models()}</th><th>{m.gateway_concurrency()}</th><th>{m.connection_cooldown()}</th><th>{m.connection_failures_heading()}</th><th><span class="sr-only">{m.connection_test()}</span></th></tr></thead><tbody>{#each connectionsForAccount(a) as conn (conn.id)}<tr><td class="mono">{conn.id}</td><td><div class="status-cell"><StatusDot status={conn.status}/><Badge status={conn.status} label={(statusLabels[conn.status] ?? m.connection_status_unknown)()}/></div></td><td class="mono">{conn.model_count}</td><td class="mono">{conn.active_requests} / {conn.max_concurrent}</td><td>{conn.cooldown_until ? cooldownRemaining(conn.cooldown_until) : m.common_none()}</td><td class="mono">{conn.failure_count ?? 0}</td><td class="test-cell">{#if testResults[conn.id]}<span class="test-badge" class:ok={testResults[conn.id].ok} class:err={!testResults[conn.id].ok}>{testResults[conn.id].ok ? `✓ ${testResults[conn.id].latency_ms}ms` : '✗'}</span>{/if}<Button size="sm" variant="outline" disabled={testing === conn.id} onclick={() => testConn(conn.id)}>{#if testing === conn.id}<Spinner size="sm" />{:else}{m.connection_test()}{/if}</Button></td></tr>{/each}</tbody></table></div>
              {/if}
            </div>
          </article>
        {/each}
      </div>
    {/if}
    {#if unassignedConnections.length > 0}
      <section class="unassigned"><div class="section-header"><div><h2 class="section-title">{m.unassigned_connections()}</h2><p class="refresh-note">{m.unassigned_connections_desc()}</p></div></div><div class="table-wrap"><table><thead><tr><th>{m.connection_id()}</th><th>{m.connection_status()}</th><th>{m.connection_models()}</th><th>{m.gateway_concurrency()}</th><th>{m.connection_cooldown()}</th><th>{m.connection_failures_heading()}</th></tr></thead><tbody>{#each unassignedConnections as conn (conn.id)}<tr><td class="mono">{conn.id}</td><td><div class="status-cell"><StatusDot status={conn.status}/><Badge status={conn.status} label={(statusLabels[conn.status] ?? m.connection_status_unknown)()}/></div></td><td class="mono">{conn.model_count}</td><td class="mono">{conn.active_requests} / {conn.max_concurrent}</td><td>{conn.cooldown_until ? cooldownRemaining(conn.cooldown_until) : m.common_none()}</td><td class="mono">{conn.failure_count ?? 0}</td></tr>{/each}</tbody></table></div></section>
    {/if}
  {/if}
</div>

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
          <div class="form-fields">
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
  .loading { display: flex; align-items: center; gap: 8px; color: var(--text-3); font-size: 0.875rem; padding: 16px 0; }
  .back { color: var(--text-3); font-size: 0.875rem; text-decoration: none; display: inline-flex; align-items: center; gap: 4px; margin-bottom: 1rem; }
  .back:hover { color: var(--accent); }
  .header { margin-bottom: 2rem; }
  .provider-head { display: flex; align-items: flex-start; gap: 16px; margin-top: 0.75rem; }
  .icon { align-items: center; border-radius: 12px; color: #fff; display: flex; flex-shrink: 0; font-size: 1.5rem; font-weight: 700; height: 56px; justify-content: center; width: 56px; }
  .name-row { display: flex; align-items: center; gap: 8px; }
  .page-title { margin: 0; }
  .site-link { color: var(--text-3); display: flex; }
  .site-link:hover { color: var(--accent); }
  .desc { color: var(--text-3); font-size: 0.875rem; margin: 4px 0 0; }
  .section-header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 0.75rem; }
  .header-actions { display: flex; gap: 8px; }
  .section-title { font-size: 1rem; font-weight: 600; color: var(--text-1); margin: 0; }
  .refresh-note { color: var(--text-3); font-size: 0.75rem; margin: 0 0 0.75rem; }
  .muted-note { color: var(--text-3); font-size: 0.875rem; }
  .mono { font-family: ui-monospace, 'SF Mono', Menlo, monospace; font-size: 0.8125rem; }
  .status-cell { display: flex; align-items: center; gap: 6px; }
  .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; }
  .accounts-stack { display: grid; gap: 16px; }
  .account-panel { border: 1px solid var(--border); background: var(--bg-surface); }
  .account-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 16px; padding: 15px 16px; border-bottom: 1px solid var(--border); }
  .account-head h3 { margin: 2px 0; font-size: var(--text-md); }
  .eyebrow { color: var(--text-3); font-size: var(--text-2xs); text-transform: uppercase; letter-spacing: .07em; }
  .account-id { color: var(--text-3); font-size: var(--text-2xs); }
  .account-actions { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; justify-content: flex-end; }
  .account-facts { display: grid; grid-template-columns: repeat(3, 1fr); margin: 0; border-bottom: 1px solid var(--border); }
  .account-facts div { padding: 11px 16px; border-right: 1px solid var(--border); }
  .account-facts div:last-child { border: 0; }
  .account-facts dt { color: var(--text-3); font-size: var(--text-2xs); text-transform: uppercase; letter-spacing: .05em; }
  .account-facts dd { margin: 4px 0 0; color: var(--text-1); font-size: var(--text-sm); }
  .limits { padding: 11px 16px; background: var(--bg-inset); border-bottom: 1px solid var(--border); display: flex; flex-direction: column; gap: 8px; }
  .limits-head { display: flex; align-items: center; gap: 8px; color: var(--text-1); font-size: var(--text-sm); font-weight: 600; }
  .plan-name { color: var(--text-3); font-size: var(--text-xs); font-weight: 500; text-transform: none; }
  .limits p { margin: 0; font-size: var(--text-xs); color: var(--text-2); }
  .limits-foot { display: flex; justify-content: space-between; gap: 8px; font-size: var(--text-2xs); color: var(--text-3); }
  .limits-foot .checked-at { color: var(--text-3); }
  .dup-warning { display: flex; align-items: center; gap: 6px; margin-top: 6px; font-size: var(--text-xs); color: var(--warning); }
  .connections-block h4 { margin: 0; padding: 10px 16px; color: var(--text-2); font-size: var(--text-2xs); text-transform: uppercase; letter-spacing: .06em; }
  .connections-block .muted-note { padding: 0 16px 14px; }
  .table-wrap { overflow-x: auto; }
  .unassigned { margin-top: 24px; border: 1px solid var(--border); background: var(--bg-surface); }
  .unassigned .section-header { padding: 12px 15px 0; }
  @media (max-width: 700px) { .account-head { flex-direction: column; } .account-actions { justify-content: flex-start; } .account-facts { grid-template-columns: 1fr; } .account-facts div { border-right: 0; border-bottom: 1px solid var(--border); } .header-actions { flex-wrap: wrap; justify-content: flex-end; } }
  :global(.dialog-overlay) { position: fixed; inset: 0; background: rgba(0,0,0,.45); z-index: 50; }
  :global(.dialog-content) { position: fixed; top: 50%; left: 50%; transform: translate(-50%,-50%); z-index: 51; background: var(--bg-surface); border: 1px solid var(--border); border-radius: var(--radius); width: min(480px,calc(100vw - 32px)); box-shadow: 0 8px 32px rgba(0,0,0,.25); }
  :global(.dialog-title) { font-size: 1rem; font-weight: 600; color: var(--text-1); margin: 0; }
  :global(.dialog-close) { display: flex; background: transparent; border: none; color: var(--text-3); cursor: pointer; padding: 4px; border-radius: var(--radius-sm); }
  :global(.dialog-header) { display: flex; align-items: center; justify-content: space-between; padding: 20px 20px 0; }
  :global(.dialog-footer) { display: flex; justify-content: flex-end; gap: 8px; padding: 0 20px 20px; }
  .form-fields { display: flex; flex-direction: column; gap: 14px; padding: 16px 20px; }
  .field { display: flex; flex-direction: column; gap: 5px; }
  .field label { font-size: 0.8125rem; font-weight: 500; color: var(--text-2); }
  .field input { background: var(--bg-elevated); border: 1px solid var(--border); border-radius: var(--radius-sm); color: var(--text-1); font-size: 0.875rem; padding: 0.4375rem 0.625rem; }
  .field input:focus { border-color: var(--accent); outline: none; }
  .field-hint { font-size: 0.75rem; color: var(--text-3); }
  .yaml-step { display: flex; flex-direction: column; gap: 12px; padding: 16px 20px; }
  .yaml-note { font-size: 0.875rem; color: var(--text-2); margin: 0; }
  .yaml-block { position: relative; background: var(--bg-base); border: 1px solid var(--border); border-radius: var(--radius-sm); padding: 12px; }
  .yaml-code { font-family: ui-monospace, 'SF Mono', Menlo, monospace; font-size: 0.8125rem; line-height: 1.5; margin: 0; white-space: pre; overflow-x: auto; }
  .yaml-env-note { font-size: 0.8125rem; color: var(--text-3); margin: 0; }
  .yaml-env-note code { font-family: ui-monospace, 'SF Mono', Menlo, monospace; }
  .test-cell { text-align: right; white-space: nowrap; display: flex; align-items: center; gap: 6px; justify-content: flex-end; }
  .test-badge { border-radius: 9999px; font-size: 0.75rem; font-weight: 500; padding: 2px 8px; }
  .test-badge.ok { background: color-mix(in oklch, var(--success) 15%, transparent); color: var(--success); }
  .test-badge.err { background: color-mix(in oklch, var(--danger) 15%, transparent); color: var(--danger); }
</style>
