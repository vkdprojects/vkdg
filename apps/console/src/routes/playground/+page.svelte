<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api.js';
  import type { ConnectionSummary } from '$lib/api.js';
  import { Button, EmptyState, Input, Select, Spinner, Stat } from '$lib/components/index.js';
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
  /** Prompt that produced the current turn, shown as the user bubble. */
  let sentMessage = $state('');

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
    sentMessage = userMessage;
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
    <h1>{m.playground_heading()}</h1>
  </div>

  <div class="panels">
    <!-- Left: config -->
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

    <!-- Right: chat -->
    <section class="panel chat" class:has-error={!!error} aria-label={m.playground_heading()}>
      <div class="thread" aria-live="polite">
        {#if !hasResult && !sending && !error}
          <EmptyState
            title={m.playground_empty_title()}
            description={m.playground_empty()}
          />
        {:else}
          <div class="bubble-row user">
            <div class="bubble bubble-user">{sentMessage}</div>
          </div>

          {#if error}
            <div class="bubble-row assistant">
              <div class="bubble bubble-error" role="alert">
                <p class="error-label">{m.common_error()}</p>
                <pre class="error-body mono">{error}</pre>
              </div>
            </div>
          {:else}
            <div class="bubble-row assistant">
              <div class="bubble bubble-assistant">
                {#if sending && !response}
                  <div class="loading-hint"><Spinner size="sm" /> {m.playground_waiting()}</div>
                {:else}
                  <pre class="response-text">{response}<span class="cursor" class:visible={sending}>▌</span></pre>
                {/if}
              </div>
            </div>

            {#if ttft !== null || duration !== null || tokenCount !== null}
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
          {/if}
        {/if}
      </div>

      <!-- Sticky composer -->
      <div class="composer">
        <label class="sr-only" for="user-message">{m.playground_message_label()}</label>
        <textarea
          id="user-message"
          bind:value={userMessage}
          rows={2}
          placeholder="Say something…"
          required
        ></textarea>
        <Button
          variant="primary"
          size="md"
          disabled={sending || !userMessage.trim()}
          onclick={send}
        >
          {#if sending}<Spinner size="sm" />{/if}
          {m.playground_send()}
          {#if !sending}<SendHorizonal size={14} aria-hidden="true" />{/if}
        </Button>
      </div>
    </section>
  </div>
</div>

<style>
  .panels {
    display: grid;
    grid-template-columns: minmax(calc(var(--space-8) * 4.5), calc(var(--space-8) * 6)) minmax(0, 1fr);
    gap: var(--space-5);
    align-items: start;
  }

  @media (max-width: 900px) {
    .panels { grid-template-columns: minmax(0, 1fr); }
  }

  /* Config */
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

  /* Chat */
  .chat {
    display: flex;
    flex-direction: column;
    height: min(78vh, calc(var(--space-8) * 13.5));
    min-height: calc(var(--space-8) * 7);
  }

  .chat.has-error { border-color: color-mix(in oklch, var(--danger) 55%, var(--border)); }

  .thread {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
    padding: var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .bubble-row { display: flex; }
  .bubble-row.user { justify-content: flex-end; }
  .bubble-row.assistant { justify-content: flex-start; }

  .bubble {
    max-width: min(85%, 70ch);
    padding: var(--space-3) var(--space-4);
    font-size: var(--text-base);
    line-height: var(--leading);
    overflow-wrap: anywhere;
    white-space: pre-wrap;
  }

  .bubble-user {
    background: var(--accent);
    color: var(--on-accent);
    border-radius: var(--radius-lg) var(--radius-lg) var(--radius-sm) var(--radius-lg);
    box-shadow: var(--shadow-1);
  }

  .bubble-assistant {
    background: var(--bg-inset);
    border: var(--border-w) solid var(--border);
    color: var(--text-1);
    border-radius: var(--radius-lg) var(--radius-lg) var(--radius-lg) var(--radius-sm);
  }

  .bubble-error {
    background: var(--danger-subtle);
    border: var(--border-w) solid color-mix(in oklch, var(--danger) 35%, transparent);
    border-radius: var(--radius-lg) var(--radius-lg) var(--radius-lg) var(--radius-sm);
  }

  .response-text {
    font: inherit;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    color: inherit;
    margin: 0;
  }

  .cursor { opacity: 0; color: var(--accent); }
  .cursor.visible { opacity: 1; animation: blink var(--dur-blink) step-end infinite; }

  @keyframes blink {
    50% { opacity: 0; }
  }

  @media (prefers-reduced-motion: reduce) {
    .cursor.visible { animation: none; }
  }

  .loading-hint {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-3);
    font-size: var(--text-sm);
  }

  .meta-row {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(calc(var(--space-8) * 2), 1fr));
    gap: var(--space-4);
    padding: var(--space-3) var(--space-4);
    background: var(--bg-inset);
    border: var(--border-w) solid var(--border);
    border-radius: var(--radius);
  }

  .meta-row :global(.stat-value) { font-size: var(--text-xl); }

  .error-label {
    font-size: var(--text-sm);
    font-weight: var(--weight-semibold);
    color: var(--danger);
    margin: 0 0 var(--space-2);
  }

  .error-body {
    font-size: var(--text-xs);
    color: var(--danger);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    margin: 0;
  }

  /* Sticky composer */
  .composer {
    position: sticky;
    bottom: 0;
    display: flex;
    align-items: flex-end;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    background: var(--bg-surface);
    border-top: var(--border-w) solid var(--border);
  }

  .composer textarea {
    flex: 1;
    min-width: 0;
    resize: none;
    max-height: calc(var(--space-8) * 2.25);
    field-sizing: content;
    border-radius: var(--radius);
    font-size: var(--text-base);
  }

  .composer :global(.btn) { min-height: var(--control-h); }

  @media (max-width: 900px) {
    .chat { height: auto; min-height: 0; max-height: none; }
    .thread { min-height: calc(var(--space-8) * 4); max-height: 60vh; }
    .composer { position: sticky; bottom: 0; z-index: var(--z-raised); }
  }

  @media (max-width: 480px) {
    .thread { padding: var(--space-4); }
    .bubble { max-width: 94%; }
    .composer { flex-direction: column; align-items: stretch; }
  }
</style>
