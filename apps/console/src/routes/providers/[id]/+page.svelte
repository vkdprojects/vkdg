<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import { ExternalLink, PlusIcon, XIcon, RefreshCwIcon } from 'lucide-svelte';
  import { Dialog } from 'bits-ui';
  import { api } from '$lib/api.js';
  import type { Account, ConnectionStatus, ConnectionSummary, ConnectionTestResult, OAuthProvider } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { Badge, Button, CopyButton, EmptyState, Select, Spinner, StatusDot } from '$lib/components/index.js';
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

    <!-- ── Connections (health) ─────────────────────────────────────── -->
    <div class="section-header">
      <h2 class="section-title">{m.nav_connections()}</h2>
      <div class="header-actions">
        <Button variant="outline" size="sm" onclick={load} disabled={refreshing} ariaLabel={m.common_refresh()}>
          <RefreshCwIcon size={14} aria-hidden="true" /> {m.common_refresh()}
        </Button>
        <Button size="sm" onclick={openConnDialog}>
          <PlusIcon size={14} aria-hidden="true" /> {m.connection_add()}
        </Button>
      </div>
    </div>
    {#if updatedAt}<p class="refresh-note">{m.connection_auto_refresh()} {m.common_updated_at({ time: timeFmt.format(updatedAt) })}</p>{/if}

    {#if connections.length === 0}
      <p class="muted-note">{m.connection_empty()}</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th scope="col">{m.connection_id()}</th>
            <th scope="col">{m.connection_status()}</th>
            <th scope="col">{m.connection_models()}</th>
            <th scope="col">{m.connection_active_requests()}</th>
            <th scope="col">{m.connection_cooldown()}</th>
            <th scope="col"><span class="sr-only">Test</span></th>
          </tr>
        </thead>
        <tbody>
          {#each connections as conn (conn.id)}
            <tr>
              <td class="mono">{conn.id}</td>
              <td>
                <div class="status-cell">
                  <StatusDot status={conn.status} />
                  <Badge status={conn.status} label={(statusLabels[conn.status] ?? m.connection_status_unknown)()} />
                </div>
              </td>
              <td>{conn.model_count}</td>
              <td class="mono">{conn.active_requests}/{conn.max_concurrent}</td>
              <td class="cooldown-cell">
                {#if conn.cooldown_until}
                  <div>{m.connection_cooldown_until({ time: timeFmt.format(new Date(conn.cooldown_until)) })}</div>
                  {#if conn.failure_count != null}<div class="hint">{m.connection_failures({ n: conn.failure_count })}</div>{/if}
                {:else}
                  {m.common_none()}
                {/if}
              </td>
              <td class="test-cell">
                {#if testResults[conn.id]}
                  <span class="test-badge" class:ok={testResults[conn.id].ok} class:err={!testResults[conn.id].ok}
                    title={testResults[conn.id].error ?? `${testResults[conn.id].latency_ms}ms`}>
                    {testResults[conn.id].ok ? `✓ ${testResults[conn.id].latency_ms}ms` : '✗ fail'}
                  </span>
                {/if}
                <Button size="sm" variant="outline" disabled={testing === conn.id}
                  onclick={() => testConn(conn.id)}>
                  {#if testing === conn.id}<Spinner size="sm" />{:else}Test{/if}
                </Button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}

    <!-- ── Accounts ─────────────────────────────────────────────────── -->
    {#if accounts.length > 0 || provider.category === 'oauth_ide'}
      <div class="section-header" style="margin-top: 2rem;">
        <h2 class="section-title">{m.provider_detail_accounts()}</h2>
        <Button size="sm" onclick={() => connect()}>{m.provider_detail_connect()}</Button>
      </div>

      {#if accounts.length === 0}
        <EmptyState title={m.provider_detail_no_accounts()} description={m.providers_connect_first()} />
      {:else}
        <table>
          <thead>
            <tr>
              <th scope="col">{m.acct_label()}</th>
              <th scope="col">{m.acct_expires()}</th>
              <th scope="col">{m.acct_status()}</th>
              <th scope="col"><span class="sr-only">{m.common_actions()}</span></th>
            </tr>
          </thead>
          <tbody>
            {#each accounts as a (a.id)}
              <tr>
                <td class="label-cell">{a.label}</td>
                <td class="date-cell">{a.expires_at ? formatDate(a.expires_at) : m.acct_never()}</td>
                <td>
                  {#if a.status === 'needs_login'}
                    <span class="badge warn">{m.acct_status_needs_login()}</span>
                  {:else}
                    <span class="badge ok">{m.acct_status_active()}</span>
                  {/if}
                </td>
                <td class="action-cell">
                  <Button variant={a.status === 'needs_login' ? 'primary' : 'outline'} size="sm"
                    onclick={() => connect(a.id)}>{m.acct_reauth()}</Button>
                  <Button variant="danger" size="sm" disabled={deleting === a.id}
                    onclick={() => deleteAccount(a)}>{m.acct_delete()}</Button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
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
  .cooldown-cell .hint { color: var(--text-3); font-size: 0.75rem; }
  .label-cell { font-weight: 500; color: var(--text-1); }
  .date-cell { font-size: 0.8125rem; white-space: nowrap; }
  .action-cell { text-align: right; white-space: nowrap; }
  .action-cell :global(.btn + .btn) { margin-left: 6px; }
  .badge { display: inline-flex; padding: 2px 8px; border-radius: 9999px; font-size: 0.75rem; font-weight: 500; }
  .badge.ok { background: color-mix(in oklch, var(--success) 15%, transparent); color: var(--success); }
  .badge.warn { background: color-mix(in oklch, var(--warning) 15%, transparent); color: var(--warning); }
  .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; }
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
