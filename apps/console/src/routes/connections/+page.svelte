<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api.js';
  import type { ConnectionSummary } from '$lib/api.js';
  import { Badge, StatusDot, EmptyState, Button, Input, Select, Spinner, CopyButton } from '$lib/components/index.js';
  import { m } from '$lib/paraglide/messages.js';
  import { Dialog } from 'bits-ui';
  import { PlusIcon, XIcon, CheckIcon } from 'lucide-svelte';
  import { toast } from 'svelte-sonner';

  let connections = $state<ConnectionSummary[]>([]);
  let loading = $state(true);

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
  };

  $effect(() => {
    if (provider in defaultEnvVar) envVar = defaultEnvVar[provider];
    if (provider in defaultModels) models = defaultModels[provider];
    if (!connId) connId = `${provider}-default`;
  });

  // Generate the YAML snippet the user needs to paste into vkdg.yaml
  const yamlSnippet = $derived(() => {
    const idLine     = `  - id: ${connId || provider + '-default'}`;
    const provLine   = `    provider: ${provider}`;
    const urlLine    = showBaseUrl && baseUrl ? `    base_url: ${baseUrl}\n` : '';
    const envLine    = `    auth:\n      type: api_key\n      env_var: ${envVar || 'API_KEY'}`;
    const modelList  = (models || '*').split(',').map(m => m.trim()).map(m => `"${m}"`).join(', ');
    const modelsLine = `    models: [${modelList}]`;
    const miscLines  = `    max_concurrent: 50\n    weight: 1`;
    return `connections:\n${idLine}\n${provLine}\n${urlLine}    ${envLine}\n${modelsLine}\n${miscLines}`;
  });

  onMount(async () => {
    try {
      const res = await api.listConnections();
      connections = res.items;
    } catch (e) {
      toast.error((e as Error).message);
    } finally {
      loading = false;
    }
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
    <h1 class="page-title">{m.nav_connections()}</h1>
    <Button variant="primary" size="sm" onclick={openDialog}>
      <PlusIcon size={14} />
      {m.connection_add()}
    </Button>
  </div>

  {#if loading}
    <div class="loading"><Spinner size="sm" /> Loading…</div>
  {:else if connections.length === 0}
    <EmptyState
      title={m.connection_empty()}
      description="Add a provider connection to start routing requests."
    />
  {:else}
    <table>
      <thead>
        <tr>
          <th scope="col">{m.connection_id()}</th>
          <th scope="col">{m.connection_provider()}</th>
          <th scope="col">{m.connection_status()}</th>
          <th scope="col">{m.connection_models()}</th>
          <th scope="col">{m.connection_active_requests()}</th>
        </tr>
      </thead>
      <tbody>
        {#each connections as conn (conn.id)}
          <tr>
            <td class="mono">{conn.id}</td>
            <td>{conn.provider}</td>
            <td>
              <div class="status-cell">
                <StatusDot status={conn.status} />
                <Badge status={conn.status} />
              </div>
            </td>
            <td>{conn.model_count}</td>
            <td>{conn.active_requests}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

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
          <div class="form-fields">
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
    gap: 8px;
    color: var(--text-3);
    font-size: 0.875rem;
    padding: 32px 0;
  }

  :global(.dialog-overlay) {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.45);
    z-index: 50;
    animation: fadeIn 0.15s ease;
  }

  :global(.dialog-content) {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    z-index: 51;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    width: min(480px, calc(100vw - 32px));
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.25);
    animation: slideIn 0.15s ease;
  }

  :global(.dialog-title) {
    font-size: 1rem;
    font-weight: 600;
    color: var(--text-1);
    margin: 0;
  }

  :global(.dialog-close) {
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: none;
    color: var(--text-3);
    cursor: pointer;
    padding: 4px;
    border-radius: var(--radius-sm);
    transition: color 0.1s, background 0.1s;
  }

  :global(.dialog-close:hover) {
    color: var(--text-1);
    background: var(--bg-hover);
  }

  .dialog-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 20px 20px 0;
  }

  .form-fields {
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 20px;
  }

  .dialog-footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 0 20px 20px;
  }

  .status-cell {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .mono {
    font-family: monospace;
    font-size: 0.8125rem;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .field label {
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--text-2);
  }

  .field input {
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--text-1);
    font-size: 0.875rem;
    padding: 0.4375rem 0.625rem;
    transition: border-color 0.15s;
    width: 100%;
    box-sizing: border-box;
  }

  .field input:focus {
    border-color: var(--accent);
    outline: none;
  }

  .field input::placeholder {
    color: var(--text-3);
  }

  @keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  @keyframes slideIn {
    from { opacity: 0; transform: translate(-50%, -48%); }
    to { opacity: 1; transform: translate(-50%, -50%); }
  }
</style>
