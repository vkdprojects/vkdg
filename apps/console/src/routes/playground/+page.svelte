<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api.js';
  import type { ConnectionSummary } from '$lib/api.js';
  import { Button, Card, EmptyState, Input, Select, Spinner, Stat } from '$lib/components/index.js';
  import { m } from '$lib/paraglide/messages.js';
  import { SendHorizonal, ChevronDown, ChevronUp } from 'lucide-svelte';
  import { toast } from 'svelte-sonner';

  const GATEWAY = import.meta.env.VITE_GATEWAY_URL ?? 'http://localhost:8080';

  let connections = $state<ConnectionSummary[]>([]);
  let loadingConnections = $state(true);

  let apiKey = $state('');
  let selectedConnection = $state('');
  let model = $state('');
  let systemPrompt = $state('');
  let systemOpen = $state(false);
  let userMessage = $state('');
  let temperature = $state(0.7);
  let useStreaming = $state(true);

  let sending = $state(false);
  let response = $state('');
  let error = $state('');
  let ttft = $state<number | null>(null);
  let duration = $state<number | null>(null);
  let tokenCount = $state<number | null>(null);
  let hasResult = $state(false);

  const connectionOptions = $derived(
    connections.map((conn) => ({ value: conn.id, label: `${conn.id} (${conn.provider})` })),
  );

  onMount(async () => {
    try {
      const res = await api.listConnections();
      connections = res.items;
      if (connections.length > 0) selectedConnection = connections[0].id;
    } catch (e) {
      toast.error((e as Error).message);
    } finally {
      loadingConnections = false;
    }
  });

  async function send() {
    if (!userMessage.trim()) return;
    sending = true;
    error = '';
    response = '';
    ttft = null;
    duration = null;
    tokenCount = null;
    hasResult = false;

    const start = Date.now();
    const body = {
      model: model.trim() || 'default',
      messages: [
        ...(systemPrompt.trim() ? [{ role: 'system', content: systemPrompt }] : []),
        { role: 'user', content: userMessage },
      ],
      temperature,
      stream: useStreaming,
    };

    const headers: Record<string, string> = { 'Content-Type': 'application/json' };
    if (apiKey.trim()) headers['Authorization'] = `Bearer ${apiKey.trim()}`;

    let resp: Response;
    try {
      resp = await fetch(`${GATEWAY}/v1/chat/completions`, {
        method: 'POST',
        headers,
        body: JSON.stringify(body),
      });
    } catch (e) {
      error = String(e);
      sending = false;
      return;
    }

    if (!resp.ok) {
      error = `${resp.status} ${await resp.text()}`;
      sending = false;
      return;
    }

    if (useStreaming) {
      const reader = resp.body!.getReader();
      const decoder = new TextDecoder();
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        if (ttft === null) ttft = Date.now() - start;
        const chunk = decoder.decode(value, { stream: true });
        for (const line of chunk.split('\n')) {
          if (!line.startsWith('data: ')) continue;
          const data = line.slice(6).trim();
          if (data === '[DONE]') continue;
          try {
            const parsed = JSON.parse(data);
            const delta = parsed.choices?.[0]?.delta?.content ?? '';
            response += delta;
            if (parsed.usage?.completion_tokens) {
              tokenCount = parsed.usage.completion_tokens;
            }
          } catch { /* skip malformed chunks */ }
        }
      }
    } else {
      const json = await resp.json();
      ttft = Date.now() - start;
      response = json.choices?.[0]?.message?.content ?? JSON.stringify(json, null, 2);
      if (json.usage?.completion_tokens) tokenCount = json.usage.completion_tokens;
    }

    duration = Date.now() - start;
    hasResult = true;
    sending = false;
  }
</script>

<div class="page playground">
  <div class="page-header">
    <h1 class="page-title">{m.playground_heading()}</h1>
  </div>

  <div class="panels">
    <!-- Left: controls -->
    <Card padding="1.25rem">
      <aside class="controls">
        <Input
          id="api-key"
          type="password"
          label={m.playground_api_key()}
          bind:value={apiKey}
          placeholder="vkdg_…"
          autocomplete="off"
        />
        <p class="hint">{m.playground_api_key_hint()}</p>

        <div class="field-group">
          <span class="field-label">{m.playground_connection()} <span class="optional">optional</span></span>
          {#if loadingConnections}
            <p class="hint"><Spinner size="sm" /> {m.common_loading()}</p>
          {:else if connections.length === 0}
            <p class="hint">{m.connection_empty()}</p>
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
        <div class="field-group">
          <button
            type="button"
            class="collapse-toggle"
            onclick={() => (systemOpen = !systemOpen)}
            aria-expanded={systemOpen}
          >
            {m.playground_system_prompt()}
            {#if systemOpen}<ChevronUp size={14} />{:else}<ChevronDown size={14} />{/if}
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

        <div class="field-group">
          <label for="user-message">{m.playground_message_label()}</label>
          <textarea
            id="user-message"
            class="mono"
            bind:value={userMessage}
            rows={6}
            placeholder="Say something…"
            required
          ></textarea>
        </div>

        <div class="field-group">
          <label for="temperature">
            {m.playground_temperature()}
            <span class="value-badge mono">{temperature.toFixed(1)}</span>
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

        <div class="field-group toggle-row">
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

        <Button
          variant="primary"
          size="lg"
          disabled={sending || !userMessage.trim()}
          onclick={send}
        >
          {#if sending}<Spinner size="sm" />{/if}
          {m.playground_send()}
          {#if !sending}<SendHorizonal size={14} aria-hidden="true" />{/if}
        </Button>
      </aside>
    </Card>

    <!-- Right: response -->
    <Card padding="1.25rem">
      <section class="output" class:has-error={!!error}>
        {#if !hasResult && !sending && !error}
          <EmptyState
            title={m.playground_empty_title()}
            description={m.playground_empty()}
          />
        {:else if error}
          <div class="error-box">
            <p class="error-label">{m.common_error()}</p>
            <pre class="error-body mono">{error}</pre>
          </div>
        {:else}
          {#if sending && !response}
            <div class="loading-hint"><Spinner size="sm" /> {m.playground_waiting()}</div>
          {:else}
            <pre class="response-text mono">{response}<span class="cursor" class:visible={sending}>▌</span></pre>
          {/if}

          <div class="meta-row">
            {#if ttft !== null}
              <Stat label={m.playground_ttft()} value={ttft} unit="ms" />
            {/if}
            {#if duration !== null}
              <Stat label={m.playground_total()} value={duration} unit="ms" />
            {/if}
            {#if tokenCount !== null}
              <Stat label={m.playground_tokens()} value={tokenCount} />
            {/if}
          </div>
        {/if}
      </section>
    </Card>
  </div>
</div>

<style>
  .playground { max-width: 1200px; }

  .page-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 1.5rem;
  }

  .page-title { margin: 0; }

  .panels {
    display: flex;
    gap: 24px;
    align-items: flex-start;
  }

  .controls {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  :global(.panels > :first-child) {
    width: 40%;
    flex-shrink: 0;
  }

  :global(.panels > :last-child) {
    flex: 1;
  }

  .output {
    min-height: 480px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    position: relative;
  }

  .output.has-error {
    margin: -1px;
    border: 1px solid var(--danger);
    border-radius: var(--radius);
    padding: calc(1.25rem - 1px);
  }

  .field-group {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .hint {
    font-size: 0.8125rem;
    color: var(--text-3);
    margin: 0;
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .optional {
    font-weight: 400;
    color: var(--text-3);
    font-size: 0.75rem;
  }

  .field-group .field-label,
  .field-group label {
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--text-2);
  }

  .field-group textarea {
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--text-1);
    font-size: 0.875rem;
    padding: 0.4375rem 0.625rem;
    transition: border-color 0.15s;
    width: 100%;
    box-sizing: border-box;
    font-family: inherit;
    resize: vertical;
  }

  .field-group textarea:focus {
    border-color: var(--accent);
    outline: none;
  }

  .field-group textarea::placeholder {
    color: var(--text-3);
  }

  .collapse-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    background: none;
    border: none;
    padding: 0;
    cursor: pointer;
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--text-2);
  }

  .collapse-toggle:hover { color: var(--text-1); }

  .value-badge {
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--accent);
    margin-left: 4px;
  }

  input[type="range"] {
    width: 100%;
    accent-color: var(--accent);
    cursor: pointer;
  }

  .range-labels {
    display: flex;
    justify-content: space-between;
    font-size: 0.6875rem;
    color: var(--text-3);
  }

  .toggle-row {
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
  }

  .toggle-label {
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--text-2);
  }

  .switch {
    width: 36px;
    height: 20px;
    border-radius: 10px;
    background: var(--border-strong);
    border: none;
    cursor: pointer;
    position: relative;
    transition: background 0.15s;
    padding: 0;
  }

  .switch.on { background: var(--accent); }

  .thumb {
    position: absolute;
    top: 3px;
    left: 3px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #fff;
    transition: transform 0.15s;
  }

  .switch.on .thumb { transform: translateX(16px); }

  .response-text {
    flex: 1;
    white-space: pre-wrap;
    word-break: break-word;
    font-size: 0.8125rem;
    color: var(--text-1);
    margin: 0;
    line-height: 1.6;
  }

  .cursor { opacity: 0; }
  .cursor.visible { opacity: 1; animation: blink 1s step-end infinite; }

  @keyframes blink {
    50% { opacity: 0; }
  }

  .loading-hint {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-3);
    font-size: 0.875rem;
  }

  .meta-row {
    display: flex;
    gap: 24px;
    flex-wrap: wrap;
    border-top: 1px solid var(--border);
    padding-top: 12px;
    margin-top: auto;
  }

  .error-box {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .error-label {
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--danger);
    margin: 0;
  }

  .error-body {
    font-size: 0.8125rem;
    color: var(--danger);
    white-space: pre-wrap;
    word-break: break-word;
    margin: 0;
    opacity: 0.85;
  }
</style>
