<script lang="ts">
  import type { PageData } from './$types';
  import { enhance } from '$app/forms';
  import { Badge, StatusDot, EmptyState, Button, Input, Select } from '$lib/components/index.js';
  import { m } from '$lib/paraglide/messages.js';
  import { Dialog } from 'bits-ui';
  import { PlusIcon, XIcon } from 'lucide-svelte';
  import { toast } from 'svelte-sonner';

  let { data }: { data: PageData } = $props();

  let dialogOpen = $state(false);
  let provider = $state('openai-compat');
  let connId = $state('');
  let baseUrl = $state('');
  let apiKey = $state('');

  const providerOptions = [
    { value: 'openai-compat', label: 'OpenAI-compatible' },
    { value: 'anthropic-compat', label: 'Anthropic-compatible' },
    { value: 'groq', label: 'Groq' },
    { value: 'gemini', label: 'Gemini' },
    { value: 'anthropic', label: 'Anthropic' },
    { value: 'openai', label: 'OpenAI' },
  ];

  const showBaseUrl = $derived(provider === 'openai-compat' || provider === 'anthropic-compat');
</script>

<div class="page">
  <div class="page-header">
    <h1 class="page-title">{m.nav_connections()}</h1>
    <Button variant="primary" size="sm" onclick={() => (dialogOpen = true)}>
      <PlusIcon size={14} />
      {m.connection_add()}
    </Button>
  </div>

  {#if data.connections.length === 0}
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
        {#each data.connections as conn (conn.id)}
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

<Dialog.Root bind:open={dialogOpen}>
  <Dialog.Portal>
    <Dialog.Overlay class="dialog-overlay" />
    <Dialog.Content class="dialog-content" aria-describedby={undefined}>
      <div class="dialog-header">
        <Dialog.Title class="dialog-title">{m.connection_add()}</Dialog.Title>
        <Dialog.Close class="dialog-close" aria-label={m.common_cancel()}>
          <XIcon size={16} />
        </Dialog.Close>
      </div>

      <form
        method="POST"
        action="?/add"
        use:enhance={() => {
          return async ({ result }) => {
            if (result.type === 'success' && result.data && !result.data.success) {
              toast.info(result.data.error as string);
            } else if (result.type === 'failure') {
              toast.error(m.common_error());
            }
            dialogOpen = false;
          };
        }}
      >
        <div class="form-fields">
          <Select
            label={m.connection_provider()}
            options={providerOptions}
            bind:value={provider}
          />

          <input type="hidden" name="provider" value={provider} />

          <div class="field">
            <label for="conn-id">{m.connection_id()}</label>
            <input id="conn-id" name="id" type="text" bind:value={connId} placeholder="my-openai" required />
          </div>

          {#if showBaseUrl}
            <div class="field">
              <label for="conn-base-url">Base URL</label>
              <input id="conn-base-url" name="base_url" type="text" bind:value={baseUrl} placeholder="https://api.openai.com/v1" />
            </div>
          {/if}

          <div class="field">
            <label for="conn-api-key">API Key</label>
            <input id="conn-api-key" name="api_key" type="password" bind:value={apiKey} placeholder="sk-…" autocomplete="off" />
          </div>
        </div>

        <div class="dialog-footer">
          <Dialog.Close>
            <Button variant="outline" type="button">{m.common_cancel()}</Button>
          </Dialog.Close>
          <Button variant="primary" type="submit">{m.connection_add()}</Button>
        </div>
      </form>
    </Dialog.Content>
  </Dialog.Portal>
</Dialog.Root>

<style>
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
