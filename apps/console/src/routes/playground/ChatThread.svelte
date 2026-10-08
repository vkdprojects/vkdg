<script lang="ts">
  import { Button, EmptyState, Spinner, Stat } from '$lib/components/index.js';
  import { m } from '$lib/paraglide/messages.js';
  import { SendHorizonal } from 'lucide-svelte';

  interface Props {
    /** Prompt that produced the current turn, shown as the user bubble. */
    sentMessage: string;
    response: string;
    error: string;
    sending: boolean;
    hasResult: boolean;
    ttft: number | null;
    duration: number | null;
    tokenCount: number | null;
    userMessage: string;
    onsend: () => void;
  }

  let {
    sentMessage,
    response,
    error,
    sending,
    hasResult,
    ttft,
    duration,
    tokenCount,
    userMessage = $bindable(),
    onsend,
  }: Props = $props();
</script>

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
      onclick={onsend}
    >
      {#if sending}<Spinner size="sm" />{/if}
      {m.playground_send()}
      {#if !sending}<SendHorizonal size={14} aria-hidden="true" />{/if}
    </Button>
  </div>
</section>

<style>
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
