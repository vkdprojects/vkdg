<script lang="ts">
  import type { PageData } from './$types';
  import { Button, EmptyState, Spinner } from '$lib/components/index.js';
  import { SendHorizonal, ChevronDown, ChevronUp } from 'lucide-svelte';

  let { data }: { data: PageData } = $props();

  let selectedConnection = $state(data.connections[0]?.id ?? '');
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
      connection_id: selectedConnection,
      model: model.trim() || undefined,
      messages: [
        ...(systemPrompt.trim() ? [{ role: 'system', content: systemPrompt }] : []),
        { role: 'user', content: userMessage },
      ],
      temperature,
      stream: useStreaming,
    };

    let resp: Response;
    try {
      resp = await fetch('/playground/chat', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
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
  <h1>Playground</h1>

  <div class="panels">
    <!-- Left: controls -->
    <aside class="controls">
      <div class="field-group">
        <label for="connection">Connection</label>
        {#if data.connections.length === 0}
          <p class="hint">No connections available.</p>
        {:else}
          <select id="connection" bind:value={selectedConnection}>
            {#each data.connections as conn (conn.id)}
              <option value={conn.id}>{conn.id} ({conn.provider})</option>
            {/each}
          </select>
        {/if}
      </div>

      <div class="field-group">
        <label for="model">Model <span class="optional">optional</span></label>
        <input id="model" type="text" bind:value={model} placeholder="e.g. claude-3-5-sonnet-20241022" />
      </div>

      <!-- Collapsible system prompt -->
      <div class="field-group">
        <button
          type="button"
          class="collapse-toggle"
          onclick={() => (systemOpen = !systemOpen)}
          aria-expanded={systemOpen}
        >
          System prompt
          {#if systemOpen}<ChevronUp size={14} />{:else}<ChevronDown size={14} />{/if}
        </button>
        {#if systemOpen}
          <textarea
            id="system-prompt"
            bind:value={systemPrompt}
            rows={4}
            placeholder="You are a helpful assistant."
            aria-label="System prompt"
          ></textarea>
        {/if}
      </div>

      <div class="field-group">
        <label for="user-message">User message</label>
        <textarea
          id="user-message"
          bind:value={userMessage}
          rows={6}
          placeholder="Say something…"
          required
        ></textarea>
      </div>

      <div class="field-group">
        <label for="temperature">
          Temperature
          <span class="value-badge">{temperature.toFixed(1)}</span>
        </label>
        <input
          id="temperature"
          type="range"
          min="0"
          max="1"
          step="0.1"
          bind:value={temperature}
        />
        <div class="range-labels">
          <span>0.0</span><span>1.0</span>
        </div>
      </div>

      <div class="field-group toggle-row">
        <span class="toggle-label">Streaming</span>
        <button
          type="button"
          role="switch"
          aria-checked={useStreaming}
          aria-label="Streaming"
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
        disabled={sending || !userMessage.trim() || data.connections.length === 0}
        onclick={send}
      >
        {#if sending}<Spinner size="sm" />{/if}
        Send
        {#if !sending}<SendHorizonal size={14} />{/if}
      </Button>
    </aside>

    <!-- Right: response -->
    <section class="output" class:has-error={!!error}>
      {#if !hasResult && !sending && !error}
        <EmptyState
          title="No response yet"
          description="Fill in a message and click Send to test your gateway."
        />
      {:else if error}
        <div class="error-box">
          <p class="error-label">Error</p>
          <pre class="error-body">{error}</pre>
        </div>
      {:else}
        {#if sending && !response}
          <div class="loading-hint"><Spinner size="sm" /> Waiting for response…</div>
        {:else}
          <pre class="response-text">{response}<span class="cursor" class:visible={sending}>▌</span></pre>
        {/if}

        <div class="meta-row">
          {#if ttft !== null}
            <span class="meta-chip">TTFT {ttft}ms</span>
          {/if}
          {#if duration !== null}
            <span class="meta-chip">Total {duration}ms</span>
          {/if}
          {#if tokenCount !== null}
            <span class="meta-chip">{tokenCount} tokens</span>
          {/if}
        </div>
      {/if}
    </section>
  </div>
</div>

<style>
  .playground { max-width: 1200px; }

  .panels {
    display: flex;
    gap: 24px;
    align-items: flex-start;
  }

  .controls {
    width: 40%;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .output {
    flex: 1;
    min-height: 480px;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    position: relative;
  }

  .output.has-error {
    border-color: var(--danger);
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
  }

  .optional {
    font-weight: 400;
    color: var(--text-3);
    font-size: 0.75rem;
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
    font-size: 0.875rem;
    color: var(--text-1);
    margin: 0;
    line-height: 1.6;
    font-family: inherit;
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
    gap: 8px;
    flex-wrap: wrap;
    border-top: 1px solid var(--border);
    padding-top: 10px;
    margin-top: auto;
  }

  .meta-chip {
    font-size: 0.75rem;
    color: var(--text-3);
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 2px 8px;
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
