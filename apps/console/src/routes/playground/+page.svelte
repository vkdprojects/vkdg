<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api.js';
  import type { ConnectionSummary } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { toast } from 'svelte-sonner';
  import ChatThread from './ChatThread.svelte';
  import PlaygroundConfig from './PlaygroundConfig.svelte';

  const GATEWAY = import.meta.env.VITE_GATEWAY_URL ?? 'http://localhost:8080';

  let connections = $state<ConnectionSummary[]>([]);
  let loadingConnections = $state(true);

  let apiKey = $state('');
  let selectedConnection = $state('');
  let model = $state('');
  let systemPrompt = $state('');
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
    <PlaygroundConfig
      {connections}
      {loadingConnections}
      bind:apiKey
      bind:selectedConnection
      bind:model
      bind:systemPrompt
      bind:temperature
      bind:useStreaming
    />
    <ChatThread
      {sentMessage}
      {response}
      {error}
      {sending}
      {hasResult}
      {ttft}
      {duration}
      {tokenCount}
      bind:userMessage
      onsend={send}
    />
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
</style>
