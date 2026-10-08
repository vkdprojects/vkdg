<script lang="ts">
  import { Button, Input } from '$lib/components/index.js';
  import { ArrowRight, ArrowLeft } from 'lucide-svelte';
  import { m } from '$lib/paraglide/messages.js';
  import StepShell from './StepShell.svelte';

  export interface StepApiKeyProps {
    providerLabel: string;
    /** Where to find a key; empty for a custom endpoint. */
    providerUrl: string;
    apiKey: string;
    onback: () => void;
    ontest: () => void;
  }

  let { providerLabel, providerUrl, apiKey = $bindable(), onback, ontest }: StepApiKeyProps = $props();
</script>

<StepShell variant="narrow" title={m.setup_enter_key_title({ provider: providerLabel })}>
  {#snippet sub()}
    {#if providerUrl}
      {m.setup_find_key_at()}
      <a href={providerUrl} target="_blank" rel="noopener noreferrer">{providerUrl}</a>
    {:else}
      {m.setup_custom_endpoint_hint()}
    {/if}
  {/snippet}

  <div class="form-group">
    <Input
      label={m.setup_api_key_label()}
      type="password"
      placeholder="sk-..."
      bind:value={apiKey}
      autocomplete="off"
    />
    <p class="key-note">{m.setup_key_stored_note()}</p>
  </div>

  {#snippet actions()}
    <Button variant="ghost" onclick={onback}>
      <ArrowLeft size={14} /> {m.common_back()}
    </Button>
    <Button disabled={apiKey.trim().length === 0} onclick={ontest}>
      {m.setup_test_connection()} <ArrowRight size={14} />
    </Button>
  {/snippet}
</StepShell>

<style>
  .form-group { margin-bottom: var(--space-5); }

  .key-note {
    font-size: var(--text-xs);
    color: var(--text-3);
    margin: var(--space-2) 0 0;
  }
</style>
