<script lang="ts">
  import { Dialog } from 'bits-ui';
  import { XIcon } from 'lucide-svelte';
  import { m } from '$lib/paraglide/messages.js';
  import { Button, CopyButton, Select } from '$lib/components/index.js';
  import { BASE_URL_PROVIDERS, buildConnectionYaml, defaultEnvVar, defaultModels, providerOptions } from '$lib/connection-wizard.js';

  interface Props {
    open: boolean;
    /** Provider preselected each time the dialog opens. */
    initialProvider?: string;
  }

  let { open = $bindable(false), initialProvider = 'openai-compat' }: Props = $props();

  let provider = $state('');
  let connId = $state('');
  let baseUrl = $state('');
  let envVar = $state('');
  let models = $state('');
  let step = $state<'form' | 'yaml'>('form');

  const showBaseUrl = $derived(BASE_URL_PROVIDERS.includes(provider));
  const yaml = $derived(buildConnectionYaml({ provider, id: connId, baseUrl, envVar, models }));

  // Each open starts a fresh draft on the requested provider.
  $effect(() => {
    if (!open) return;
    provider = initialProvider;
    step = 'form';
    connId = '';
    baseUrl = '';
  });

  $effect(() => {
    if (provider in defaultEnvVar) envVar = defaultEnvVar[provider];
    if (provider in defaultModels) models = defaultModels[provider];
    if (!connId) connId = `${provider}-default`;
  });
</script>

<Dialog.Root bind:open onOpenChange={(v) => { if (!v) step = 'form'; }}>
  <Dialog.Portal>
    <Dialog.Overlay class="dialog-overlay" />
    <Dialog.Content class="dialog-content" aria-describedby={undefined}>
      <div class="dialog-header">
        <Dialog.Title class="dialog-title">
          {step === 'form' ? m.connection_add() : 'Add to your config'}
        </Dialog.Title>
        <button class="dialog-close" aria-label={m.common_cancel()} onclick={() => (open = false)}>
          <XIcon size={16} aria-hidden="true" />
        </button>
      </div>

      {#if step === 'form'}
        <!-- Step 1: pick provider, enter details -->
        <form onsubmit={(e) => { e.preventDefault(); step = 'yaml'; }}>
          <div class="dialog-body fields">
            <Select label={m.connection_provider()} options={providerOptions} bind:value={provider} />

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
            <Button variant="outline" type="button" onclick={() => (open = false)}>{m.common_cancel()}</Button>
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
            <pre class="yaml-code">{yaml}</pre>
            <CopyButton text={yaml} />
          </div>
          <p class="yaml-env-note">
            Set the environment variable before starting the gateway:
            <br />
            <code>export {envVar || 'API_KEY'}=your-key-here</code>
          </p>
        </div>

        <div class="dialog-footer">
          <Button variant="outline" onclick={() => (step = 'form')}>← Back</Button>
          <Button variant="primary" onclick={() => (open = false)}>Done</Button>
        </div>
      {/if}
    </Dialog.Content>
  </Dialog.Portal>
</Dialog.Root>

<style>
  .yaml-step { display: flex; flex-direction: column; gap: var(--space-3); padding: var(--space-5); }
  .yaml-note, .yaml-env-note { margin: 0; font-size: var(--text-sm); color: var(--text-2); }
  .yaml-env-note code { font-family: var(--font-mono); }
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
</style>
