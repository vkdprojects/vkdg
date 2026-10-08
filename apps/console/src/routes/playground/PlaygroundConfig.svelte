<script lang="ts">
  import type { ConnectionSummary } from '$lib/api.js';
  import { Input, Select, Spinner } from '$lib/components/index.js';
  import { m } from '$lib/paraglide/messages.js';
  import { ChevronDown, ChevronUp } from 'lucide-svelte';

  interface Props {
    connections: ConnectionSummary[];
    loadingConnections: boolean;
    apiKey: string;
    selectedConnection: string;
    model: string;
    systemPrompt: string;
    temperature: number;
    useStreaming: boolean;
  }

  let {
    connections,
    loadingConnections,
    apiKey = $bindable(),
    selectedConnection = $bindable(),
    model = $bindable(),
    systemPrompt = $bindable(),
    temperature = $bindable(),
    useStreaming = $bindable(),
  }: Props = $props();

  let systemOpen = $state(false);

  const connectionOptions = $derived(
    connections.map((conn) => ({ value: conn.id, label: `${conn.id} (${conn.provider})` })),
  );
</script>

<aside class="panel config" aria-label={m.playground_heading()}>
  <div class="panel-body controls">
    <Input
      id="api-key"
      type="password"
      label={m.playground_api_key()}
      bind:value={apiKey}
      placeholder="vkdg_…"
      autocomplete="off"
    />
    <p class="field-hint hint-row">{m.playground_api_key_hint()}</p>

    <div class="field">
      <span class="field-label">{m.playground_connection()} <span class="optional">optional</span></span>
      {#if loadingConnections}
        <p class="field-hint hint-row"><Spinner size="sm" /> {m.common_loading()}</p>
      {:else if connections.length === 0}
        <p class="field-hint hint-row">{m.connection_empty()}</p>
      {:else}
        <Select id="connection" options={connectionOptions} bind:value={selectedConnection} />
      {/if}
    </div>

    <Input
      id="model"
      type="text"
      label={`${m.playground_model_label()} (optional)`}
      bind:value={model}
      placeholder="e.g. claude-3-5-sonnet-20241022"
    />

    <!-- Collapsible system prompt -->
    <div class="field">
      <button
        type="button"
        class="collapse-toggle"
        onclick={() => (systemOpen = !systemOpen)}
        aria-expanded={systemOpen}
      >
        {m.playground_system_prompt()}
        {#if systemOpen}<ChevronUp size={14} aria-hidden="true" />{:else}<ChevronDown size={14} aria-hidden="true" />{/if}
      </button>
      {#if systemOpen}
        <textarea
          id="system-prompt"
          bind:value={systemPrompt}
          rows={4}
          placeholder="You are a helpful assistant."
          aria-label={m.playground_system_prompt()}
        ></textarea>
      {/if}
    </div>

    <div class="field">
      <label for="temperature">
        <span class="label-row">
          {m.playground_temperature()}
          <span class="value-badge mono">{temperature.toFixed(1)}</span>
        </span>
      </label>
      <input
        id="temperature"
        type="range"
        min="0"
        max="1"
        step="0.1"
        bind:value={temperature}
      />
      <div class="range-labels mono">
        <span>0.0</span><span>1.0</span>
      </div>
    </div>

    <div class="field toggle-row">
      <span class="toggle-label">{m.playground_streaming()}</span>
      <button
        type="button"
        role="switch"
        aria-checked={useStreaming}
        aria-label={m.playground_streaming()}
        class="switch"
        class:on={useStreaming}
        onclick={() => (useStreaming = !useStreaming)}
      >
        <span class="thumb"></span>
      </button>
    </div>
  </div>
</aside>

<style>
  .config { margin-bottom: 0; }

  .controls {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .hint-row {
    margin: 0;
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .optional {
    font-weight: var(--weight-regular);
    color: var(--text-3);
    font-size: var(--text-xs);
  }

  .field label {
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    color: var(--text-2);
  }

  .label-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
  }

  .field textarea {
    resize: vertical;
    min-height: calc(var(--space-8) * 1.25);
  }

  .collapse-toggle {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    min-height: var(--control-h-sm);
    width: 100%;
    background: var(--bg-inset);
    border: var(--border-w) solid var(--border);
    border-radius: var(--radius);
    padding: 0 var(--space-3);
    cursor: pointer;
    font: inherit;
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    color: var(--text-2);
  }

  .collapse-toggle:hover {
    color: var(--text-1);
    border-color: var(--border-strong);
  }

  .value-badge {
    font-weight: var(--weight-semibold);
    color: var(--accent);
    background: var(--accent-subtle);
    border-radius: var(--radius-full);
    padding: var(--space-0) var(--space-2);
  }

  input[type="range"] {
    width: 100%;
    min-height: var(--space-5);
    accent-color: var(--accent);
    cursor: pointer;
  }

  .range-labels {
    display: flex;
    justify-content: space-between;
    font-size: var(--text-2xs);
    color: var(--text-3);
  }

  .toggle-row {
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
    min-height: var(--control-h-sm);
  }

  .toggle-label {
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    color: var(--text-2);
  }

  .switch {
    --switch-h: var(--space-5);
    --switch-pad: var(--space-0);
    --thumb: calc(var(--switch-h) - 2 * var(--switch-pad));
    width: calc(var(--switch-h) * 1.75);
    height: var(--switch-h);
    border-radius: var(--radius-full);
    background: var(--border-strong);
    border: none;
    cursor: pointer;
    position: relative;
    padding: 0;
    flex-shrink: 0;
  }

  .switch.on { background: var(--accent); }

  .thumb {
    position: absolute;
    top: var(--switch-pad);
    left: var(--switch-pad);
    width: var(--thumb);
    height: var(--thumb);
    border-radius: var(--radius-full);
    background: var(--on-accent);
    box-shadow: var(--shadow-1);
    transition: transform var(--dur-2) var(--ease-spring);
  }

  .switch.on .thumb { transform: translateX(calc(var(--switch-h) * 0.75)); }
</style>
