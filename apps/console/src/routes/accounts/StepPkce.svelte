<script lang="ts">
  import { ExternalLink } from 'lucide-svelte';
  import { m } from '$lib/paraglide/messages.js';
  import { Button, CopyButton, Spinner } from '$lib/components/index.js';

  interface Props {
    authorizeUrl: string;
    /** Popup is open and the callback page will deliver the code on its own. */
    autoCapture: boolean;
    /** Code received, exchange in progress. */
    busy: boolean;
    code: string;
    onreopen: () => void;
    onmanual: () => void;
    onsubmit: (e: Event) => void;
    oncancel: () => void;
  }

  let { authorizeUrl, autoCapture, busy, code = $bindable(), onreopen, onmanual, onsubmit, oncancel }: Props = $props();
</script>

{#if autoCapture}
  <!-- Popup is open — waiting for automatic code capture. -->
  <div class="pkce-auto">
    <div class="pkce-icon spin">⟳</div>
    <p class="step-intro">Authorize in the popup window…</p>
    <p class="muted status-row">The window opened at claude.ai. After you sign in, this dialog completes automatically.</p>
  </div>
  <div class="dialog-footer">
    <Button variant="outline" onclick={onmanual}>{m.acct_enter_code_manually()}</Button>
    <Button variant="outline" onclick={oncancel}>{m.common_cancel()}</Button>
  </div>
{:else if busy}
  <!-- Code received, exchange in progress. -->
  <div class="pkce-auto">
    <Spinner size="lg" />
    <p class="step-intro">Completing sign-in…</p>
  </div>
{:else}
  <!-- Manual paste: no loopback callback, blocked popup, or the user asked for it. -->
  <div class="pkce-section">
    <p class="step-intro">{m.acct_pkce_step()}</p>
    <div class="pkce-url-row">
      <code class="pkce-url" title={authorizeUrl}>{authorizeUrl.slice(0, 60)}…</code>
      <CopyButton text={authorizeUrl} />
    </div>
    <Button variant="outline" onclick={onreopen}>
      <ExternalLink size={14} aria-hidden="true" />
      Reopen popup
    </Button>
  </div>
  <form {onsubmit} class="fields">
    <div class="field">
      <label for="pkce-code">{m.acct_code_label()}</label>
      <input
        id="pkce-code"
        type="text"
        autocomplete="off"
        required
        placeholder="Paste the code or full callback URL"
        bind:value={code}
      />
    </div>
    <div class="dialog-footer">
      <Button variant="outline" onclick={oncancel}>{m.common_cancel()}</Button>
      <Button type="submit" disabled={busy || !code.trim()}>
        {#if busy}<Spinner size="sm" />{/if}
        {m.acct_submit_code()}
      </Button>
    </div>
  </form>
{/if}

<style>
  .pkce-section {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-3);
  }

  .pkce-url-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
    max-width: 100%;
    min-width: 0;
  }

  .pkce-url {
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .pkce-auto {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3) 0;
    text-align: center;
  }

  .pkce-icon {
    font-size: var(--text-2xl);
    line-height: 1;
  }

  .pkce-icon.spin {
    animation: spin calc(var(--dur-3) * 3) linear infinite;
    color: var(--accent);
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  @media (prefers-reduced-motion: reduce) {
    .pkce-icon.spin { animation: none; }
  }
</style>
