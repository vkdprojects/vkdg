<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api.js';
  import type { ConnectionStatus, ConnectionSummary } from '$lib/api.js';
  import { Badge, EmptyState, Button, Select, Spinner, CopyButton, Meter, Stat } from '$lib/components/index.js';
  import { m } from '$lib/paraglide/messages.js';
  import { formatTime, formatRelativeTime } from '$lib/format.js';
  import { Dialog, AlertDialog } from 'bits-ui';
  import { PlusIcon, XIcon, RefreshCwIcon } from 'lucide-svelte';
  import { toast } from 'svelte-sonner';

  let connections = $state<ConnectionSummary[]>([]);
  let loading = $state(true);
  let refreshing = $state(false);
  let updatedAt = $state<Date | null>(null);

  const statusLabels: Record<ConnectionStatus, () => string> = {
    healthy: m.connection_status_healthy,
    degraded: m.connection_status_degraded,
    circuit_open: m.connection_status_circuit_open,
    cooldown: m.connection_status_cooldown,
    unknown: m.connection_status_unknown,
  };

  const totalActive = $derived(connections.reduce((sum, conn) => sum + conn.active_requests, 0));
  const totalCapacity = $derived(connections.reduce((sum, conn) => sum + conn.max_concurrent, 0));

  function humanizeCooldown(value: string) {
    const timestamp = new Date(value).getTime();
    if (!Number.isFinite(timestamp)) return m.common_none();

    const seconds = Math.round((timestamp - Date.now()) / 1000);
    if (Math.abs(seconds) < 60) return formatRelativeTime(seconds, 'second');
    const minutes = Math.round(seconds / 60);
    if (Math.abs(minutes) < 60) return formatRelativeTime(minutes, 'minute');
    const hours = Math.round(minutes / 60);
    if (Math.abs(hours) < 24) return formatRelativeTime(hours, 'hour');
    return formatRelativeTime(Math.round(hours / 24), 'day');
  }

  let pendingDelete = $state<ConnectionSummary | null>(null);
  let deleting = $state(false);

  async function confirmDelete() {
    if (!pendingDelete) return;
    deleting = true;
    try {
      await api.deleteConnection(pendingDelete.id);
      toast.success(m.connection_deleted());
      pendingDelete = null;
      await load();
    } catch (e) {
      toast.error((e as Error).message);
    } finally {
      deleting = false;
    }
  }

  // Models chips: show a few entries, the rest go into the tooltip.
  const MODEL_CHIPS = 3;
  const isPattern = (id: string) => /[*?]/.test(id);

  let syncingId = $state<string | null>(null);

  async function syncModels(conn: ConnectionSummary) {
    syncingId = conn.id;
    try {
      const result = await api.syncConnectionModels(conn.id);
      toast.success(m.connection_models_synced({ count: result.count }));
      await load();
    } catch (e) {
      toast.error((e as Error).message);
    } finally {
      syncingId = null;
    }
  }
  let resettingId = $state<string | null>(null);

  async function resetCooldown(conn: ConnectionSummary) {
    resettingId = conn.id;
    try {
      await api.resetConnectionCooldown(conn.id);
      toast.success(m.connection_cooldown_reset({ id: conn.id }));
      await load();
    } catch (e) {
      toast.error((e as Error).message);
    } finally {
      resettingId = null;
    }
  }


  let dialogOpen = $state(false);
  let provider = $state('openai-compat');
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
    { value: 'mistral',    label: 'Mistral' },
    { value: 'together',   label: 'Together AI' },
    { value: 'fireworks',  label: 'Fireworks AI' },
    { value: 'sambanova',  label: 'SambaNova (free tier)' },
    { value: 'cerebras',   label: 'Cerebras (free tier)' },
    { value: 'nvidia-nim', label: 'NVIDIA NIM' },
    { value: 'kiro',       label: 'Kiro (Amazon Q)' },
  ];

  const showBaseUrl = $derived(provider === 'openai-compat' || provider === 'anthropic-compat');

  // Default env var per provider
  const defaultEnvVar: Record<string, string> = {
    'anthropic':     'ANTHROPIC_API_KEY',
    'openai':        'OPENAI_API_KEY',
    'groq':          'GROQ_API_KEY',
    'gemini':        'GEMINI_API_KEY',
    'deepseek':      'DEEPSEEK_API_KEY',
    'mistral':       'MISTRAL_API_KEY',
    'together':      'TOGETHER_API_KEY',
    'fireworks':     'FIREWORKS_API_KEY',
    'sambanova':     'SAMBANOVA_API_KEY',
    'cerebras':      'CEREBRAS_API_KEY',
    'nvidia-nim':    'NVIDIA_API_KEY',
    'kiro':          'KIRO_API_KEY',
    'openai-compat': 'API_KEY',
  };

  const defaultModels: Record<string, string> = {
    'anthropic':  'claude-*',
    'openai':     'gpt-*, o1-*, o3-*',
    'groq':       'llama-*, mixtral-*',
    'gemini':     'gemini-*',
    'deepseek':   'deepseek-*',
    'mistral':    'mistral-*',
    'together':   'meta-llama/*',
    'fireworks':  'accounts/*',
    'sambanova':  'Meta-Llama-*',
    'cerebras':   'llama3.1-*',
    'nvidia-nim': 'meta/llama-*',
    'kiro':       'claude-*, gpt-5.6-*, minimax-*, deepseek-*, glm-*, qwen3-*, auto',
  };

  $effect(() => {
    if (provider in defaultEnvVar) envVar = defaultEnvVar[provider];
    if (provider in defaultModels) models = defaultModels[provider];
    if (!connId) connId = `${provider}-default`;
  });

  // Generate the YAML snippet the user needs to paste into vkdg.yaml. One entry
  // per line with explicit indentation: `auth:` once rendered at 8 spaces and
  // the gateway refused the pasted file.
  const yamlSnippet = $derived(() => {
    const modelList = (models || '*').split(',').map((m) => `"${m.trim()}"`).join(', ');
    const lines = [
      'connections:',
      `  - id: ${connId || provider + '-default'}`,
      `    provider: ${provider}`,
      ...(showBaseUrl && baseUrl ? [`    base_url: ${baseUrl}`] : []),
      '    auth:',
      '      type: api_key',
      `      env_var: ${envVar || 'API_KEY'}`,
      `    models: [${modelList}]`,
      '    max_concurrent: 50',
      '    weight: 1',
    ];
    return lines.join('\n');
  });

  async function load() {
    refreshing = true;
    try {
      connections = (await api.listConnections()).items;
      updatedAt = new Date();
    } catch (e) {
      toast.error((e as Error).message);
    } finally {
      loading = false;
      refreshing = false;
    }
  }

  // Poll every 5s only while the tab is visible; refresh at once when it comes back.
  onMount(() => {
    let timer: ReturnType<typeof setInterval> | undefined;
    const sync = () => {
      clearInterval(timer);
      timer = undefined;
      if (document.visibilityState !== 'visible') return;
      load();
      timer = setInterval(load, 5000);
    };
    sync();
    document.addEventListener('visibilitychange', sync);
    return () => {
      clearInterval(timer);
      document.removeEventListener('visibilitychange', sync);
    };
  });

  function openDialog() {
    step = 'form';
    connId = '';
    baseUrl = '';
    dialogOpen = true;
  }

  function handleFormSubmit(e: Event) {
    e.preventDefault();
    step = 'yaml';
  }

  function closeDialog() {
    dialogOpen = false;
    step = 'form';
  }
</script>

<div class="page">
  <div class="page-header">
    <div>
      <h1 class="page-title">{m.nav_connections()}</h1>
      <p class="refresh-note" aria-live="polite">
        {m.connection_auto_refresh()}
        {#if updatedAt}{m.common_updated_at({ time: formatTime(updatedAt) })}{/if}
      </p>
    </div>
    <div class="page-actions">
      <Button variant="outline" size="sm" onclick={load} disabled={refreshing} ariaLabel={m.common_refresh()}>
        <RefreshCwIcon size={14} aria-hidden="true" />
        {m.common_refresh()}
      </Button>
      <Button variant="primary" size="sm" onclick={openDialog}>
        <PlusIcon size={14} />
        {m.connection_add()}
      </Button>
    </div>
  </div>

  {#if !loading && connections.length > 0}
    <section class="summary panel">
      <Stat label={m.connection_active_requests()} value={totalActive} unit={`/ ${totalCapacity}`} />
      <Meter bare value={totalActive} limit={totalCapacity} ariaLabel={m.connection_active_requests()} />
    </section>
  {/if}

  {#if loading}
    <div class="loading"><Spinner size="sm" /> {m.common_loading()}</div>
  {:else if connections.length === 0}
    <EmptyState title={m.connection_empty()} description={m.connection_empty_desc()} />
  {:else}
    <div class="conn-grid">
      {#each connections as conn (conn.id)}
        <article class="conn-card" data-status={conn.status}>
          <header class="conn-head">
            <div class="conn-title">
              <span class="mono conn-id" title={conn.id}>{conn.id}</span>
              <span class="conn-provider">{conn.provider}</span>
            </div>
            <Badge status={conn.status} label={(statusLabels[conn.status] ?? m.connection_status_unknown)()} />
          </header>

          <div class="conn-section">
            <span class="conn-label">{m.connection_active_requests()}</span>
            <Meter
              value={conn.active_requests}
              limit={conn.max_concurrent}
              valueText={`${conn.active_requests} / ${conn.max_concurrent}`}
            />
          </div>

          <div class="conn-section">
            <span class="conn-label">{m.connection_models()}</span>
            {#if conn.models.length === 0}
              <span class="hint">{m.common_none()}</span>
            {:else}
              <div class="model-chips" title={conn.models.join('\n')}>
                {#each conn.models.slice(0, MODEL_CHIPS) as id (id)}
                  <span class="model-chip" class:pattern={isPattern(id)}>{id}</span>
                {/each}
                {#if conn.models.length > MODEL_CHIPS}
                  <span class="hint">{m.connection_models_more({ n: conn.models.length - MODEL_CHIPS })}</span>
                {/if}
              </div>
            {/if}
          </div>

          <div class="conn-section">
            <span class="conn-label">{m.connection_cooldown()}</span>
            <div class="cooldown-info">
              {#if conn.cooldown_until}
                <div class="cooldown-until">{m.connection_cooldown_until({ time: humanizeCooldown(conn.cooldown_until) })}</div>
              {/if}
              {#if conn.failure_count != null}
                <div class="hint">{m.connection_failures({ n: conn.failure_count })}</div>
              {/if}
              {#if !conn.cooldown_until && conn.failure_count == null}
                <span class="hint">{m.common_none()}</span>
              {/if}
            </div>
          </div>

          <footer class="conn-actions">
            <span class="sr-only">{m.connection_actions()}</span>
            <Button
              variant="outline"
              size="sm"
              onclick={() => syncModels(conn)}
              disabled={syncingId === conn.id}
              ariaLabel={`${m.connection_sync_models()} ${conn.id}`}
            >
              <RefreshCwIcon size={14} aria-hidden="true" />
              {m.connection_sync_models()}
            </Button>
            {#if conn.status === 'cooldown' || conn.status === 'circuit_open' || conn.cooldown_until}
              <Button
                variant="outline"
                size="sm"
                onclick={() => resetCooldown(conn)}
                disabled={resettingId === conn.id}
                ariaLabel={`${m.connection_reset_cooldown()} ${conn.id}`}
              >
                <RefreshCwIcon size={14} aria-hidden="true" />
                {m.connection_reset_cooldown()}
              </Button>
            {/if}
            <Button
              variant="danger"
              size="sm"
              onclick={() => (pendingDelete = conn)}
              ariaLabel={`${m.connection_delete()} ${conn.id}`}
            >{m.connection_delete()}</Button>
          </footer>
        </article>
      {/each}
    </div>
  {/if}
</div>


<AlertDialog.Root open={pendingDelete !== null} onOpenChange={(v) => { if (!v) pendingDelete = null; }}>
  <AlertDialog.Portal>
    <AlertDialog.Overlay class="dialog-overlay" />
    <AlertDialog.Content class="dialog-content">
      <div class="confirm">
        <AlertDialog.Title class="dialog-title">{m.connection_delete_title()}</AlertDialog.Title>
        <AlertDialog.Description class="confirm-desc">
          {m.connection_delete_confirm({ id: pendingDelete?.id ?? '' })}
        </AlertDialog.Description>
        <div class="confirm-footer">
          <AlertDialog.Cancel class="confirm-btn outline">{m.common_cancel()}</AlertDialog.Cancel>
          <button type="button" class="confirm-btn danger" disabled={deleting} onclick={confirmDelete}>
            {m.connection_delete()}
          </button>
        </div>
      </div>
    </AlertDialog.Content>
  </AlertDialog.Portal>
</AlertDialog.Root>

<Dialog.Root bind:open={dialogOpen} onOpenChange={(v) => { if (!v) step = 'form'; }}>
  <Dialog.Portal>
    <Dialog.Overlay class="dialog-overlay" />
    <Dialog.Content class="dialog-content" aria-describedby={undefined}>
      <div class="dialog-header">
        <Dialog.Title class="dialog-title">
          {step === 'form' ? m.connection_add() : 'Add to your config'}
        </Dialog.Title>
        <button class="dialog-close" aria-label={m.common_cancel()} onclick={closeDialog}>
          <XIcon size={16} />
        </button>
      </div>

      {#if step === 'form'}
        <!-- Step 1: pick provider, enter details -->
        <form onsubmit={handleFormSubmit}>
          <div class="dialog-body fields">
            <Select
              label={m.connection_provider()}
              options={providerOptions}
              bind:value={provider}
            />

            <div class="field">
              <label for="conn-id">Connection ID</label>
              <input id="conn-id" type="text" bind:value={connId} placeholder="{provider}-default" required />
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
              <span class="field-hint">Variable name — the key stays in your environment, not in the config</span>
            </div>

            <div class="field">
              <label for="conn-models">Models (comma-separated globs)</label>
              <input id="conn-models" type="text" bind:value={models} placeholder="claude-*, gpt-4*" />
            </div>
          </div>

          <div class="dialog-footer">
            <Button variant="outline" type="button" onclick={closeDialog}>{m.common_cancel()}</Button>
            <Button variant="primary" type="submit">Generate config snippet →</Button>
          </div>
        </form>

      {:else}
        <!-- Step 2: show the YAML snippet to paste into vkdg.yaml -->
        <div class="yaml-step">
          <p class="yaml-note">
            Copy this into your <code>vkdg.yaml</code>. Hot-reload picks it up automatically — no restart needed.
          </p>
          <div class="yaml-block">
            <pre class="yaml-code">{yamlSnippet()}</pre>
            <CopyButton text={yamlSnippet()} />
          </div>
          <p class="yaml-env-note">
            Set the environment variable before starting the gateway:
            <br />
            <code>export {envVar || 'API_KEY'}=your-key-here</code>
          </p>
        </div>

        <div class="dialog-footer">
          <Button variant="outline" onclick={() => (step = 'form')}>← Back</Button>
          <Button variant="primary" onclick={closeDialog}>Done</Button>
        </div>
      {/if}
    </Dialog.Content>
  </Dialog.Portal>
</Dialog.Root>

<style>
  .loading {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-3);
    font-size: var(--text-sm);
    padding: var(--space-6) 0;
  }


  .refresh-note {
    margin: var(--space-1) 0 0;
    font-size: var(--text-xs);
    color: var(--text-3);
  }

  /* Fleet summary: hero metric + meter */
  .summary {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(var(--col-md), 1fr));
    align-items: center;
    gap: var(--space-5);
    padding: var(--space-5);
    margin-bottom: var(--space-5);
  }
  .summary :global(.stat-value) {
    color: var(--accent);
  }

  /* Connection cards */
  .conn-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, var(--col-xl)), 1fr));
    gap: var(--space-4);
  }

  .conn-card {
    --tone: var(--border-strong);
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-width: 0;
    padding: var(--space-5);
    background:
      linear-gradient(180deg, color-mix(in oklch, var(--tone) 7%, transparent), transparent 5rem), /* token-ok */
      var(--bg-surface);
    border: var(--border-w) solid color-mix(in oklch, var(--tone) 35%, var(--border));
    border-left: var(--indicator-w) solid var(--tone); /* token-ok */
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-1);
  }
  .conn-card[data-status='healthy'] { --tone: var(--success); }
  .conn-card[data-status='degraded'] { --tone: var(--warning); }
  .conn-card[data-status='circuit_open'] { --tone: var(--danger); }
  .conn-card[data-status='cooldown'] { --tone: var(--cooldown); }

  .conn-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .conn-title {
    display: flex;
    flex-direction: column;
    gap: var(--space-0);
    min-width: 0;
    flex: 1 1 var(--col-sm);
  }
  .conn-id {
    color: var(--text-1);
    font-weight: var(--weight-semibold);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .conn-provider {
    color: var(--text-3);
    font-size: var(--text-sm);
  }

  .conn-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }
  .conn-label {
    font-size: var(--text-xs);
    font-weight: var(--weight-medium);
    color: var(--text-3);
  }

  .cooldown-info {
    display: flex;
    flex-direction: column;
    gap: var(--space-0);
    font-size: var(--text-sm);
    color: var(--text-1);
  }
  .cooldown-until { color: var(--tone); font-weight: var(--weight-medium); }
  .conn-card[data-status='healthy'] .cooldown-until { color: var(--text-1); }

  .hint {
    font-size: var(--text-xs);
    color: var(--text-3);
  }

  .model-chips {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1);
  }
  .model-chip {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    padding: var(--space-0) var(--space-2);
    border: var(--border-w) solid var(--border);
    border-radius: var(--radius-full);
    background: var(--bg-inset);
    color: var(--text-1);
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .model-chip.pattern {
    border-style: dashed;
    color: var(--text-2);
  }

  .conn-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin-top: auto;
    padding-top: var(--space-4);
    border-top: var(--border-w) solid var(--border);
  }
  .conn-actions :global(button) { min-height: var(--control-h-sm); }
  .conn-actions :global(button:last-child) { margin-left: auto; }

  .yaml-step { display: flex; flex-direction: column; gap: var(--space-3); padding: var(--space-5); }
  .yaml-note, .yaml-env-note { margin: 0; font-size: var(--text-sm); color: var(--text-2); }
  .yaml-block {
    position: relative;
    background: var(--bg-inset);
    border: var(--border-w) solid var(--border);
    border-radius: var(--radius-sm);
    padding: var(--space-3);
  }
  .yaml-code {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    line-height: var(--leading);
    margin: 0;
    white-space: pre;
    overflow-x: auto;
    color: var(--text-1);
  }

  .confirm {
    padding: var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  :global(.confirm-desc) {
    font-size: var(--text-sm);
    color: var(--text-2);
    margin: 0;
  }
  .confirm-footer {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: var(--space-2);
  }
  :global(.confirm-btn) {
    border-radius: var(--radius-sm);
    cursor: pointer;
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    min-height: var(--control-h-sm);
    padding: var(--space-2) var(--space-4);
    border: var(--border-w) solid transparent;
  }
  :global(.confirm-btn.outline) {
    background: transparent;
    color: var(--text-2);
    border-color: var(--border-strong);
  }
  :global(.confirm-btn.outline:hover) { background: var(--bg-hover); color: var(--text-1); }
  :global(.confirm-btn.danger) {
    background: var(--danger);
    color: var(--on-accent);
  }
  :global(.confirm-btn:disabled) {
    opacity: 0.45;
    cursor: not-allowed;
  }
</style>
