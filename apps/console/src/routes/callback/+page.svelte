<script lang="ts">
  import { onMount } from 'svelte';

  // Extracted from URL params after mount (SSR-safe).
  let status = $state<'pending' | 'done' | 'error'>('pending');
  let message = $state('Completing authorization…');

  onMount(() => {
    const params = new URLSearchParams(window.location.search);
    const code = params.get('code');
    const state = params.get('state');
    const error = params.get('error');
    const errorDesc = params.get('error_description');

    if (error) {
      status = 'error';
      message = errorDesc ?? error;
      notify({ type: 'oauth_error', error, error_description: errorDesc ?? '' });
      return;
    }

    if (!code) {
      status = 'error';
      message = 'No authorization code received.';
      notify({ type: 'oauth_error', error: 'missing_code', error_description: message });
      return;
    }

    status = 'done';
    message = 'Authorization complete — you can close this window.';
    notify({ type: 'oauth_code', code, state: state ?? '' });
  });

  /** Broadcast the OAuth result to every listener strategy in order of reliability. */
  function notify(payload: Record<string, string>) {
    const msg = { source: 'vkdg_oauth_callback', ...payload };

    // 1. BroadcastChannel — most reliable, works across same-origin tabs/popups.
    try {
      const ch = new BroadcastChannel('vkdg_oauth_callback');
      ch.postMessage(msg);
      // Leave the channel open briefly so late listeners catch it.
      setTimeout(() => ch.close(), 2000);
    } catch {
      // BroadcastChannel not supported (unlikely in modern browsers).
    }

    // 2. postMessage to opener — works when this page is a popup.
    try {
      if (window.opener && !window.opener.closed) {
        window.opener.postMessage(msg, window.location.origin);
      }
    } catch {
      // Cross-origin opener — ignore.
    }

    // 3. localStorage fallback — for same-origin contexts that missed both above.
    try {
      localStorage.setItem(
        'vkdg_oauth_result',
        JSON.stringify({ ...msg, ts: Date.now() }),
      );
    } catch {
      // Private mode may deny localStorage.
    }

    // Auto-close the popup after a short delay so the user sees the confirmation.
    if (payload.type === 'oauth_code') {
      setTimeout(() => {
        try { window.close(); } catch { /* ignore */ }
      }, 800);
    }
  }
</script>

<svelte:head>
  <title>Authorization — VKDG</title>
</svelte:head>

<div class="callback-page">
  {#if status === 'pending'}
    <div class="icon spin">⟳</div>
    <p>{message}</p>
  {:else if status === 'done'}
    <div class="icon success">✓</div>
    <p>{message}</p>
  {:else}
    <div class="icon error">✗</div>
    <p class="error-msg">{message}</p>
    <p class="hint">Close this window and try again.</p>
  {/if}
</div>

<style>
  .callback-page {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    min-height: 100vh;
    gap: 12px;
    background: var(--bg-base, #0f1117);
    color: var(--text-1, #e8eaf0);
    font-family: system-ui, sans-serif;
  }

  .icon {
    font-size: 2.5rem;
    line-height: 1;
  }

  .icon.spin {
    animation: spin 1s linear infinite;
    color: var(--accent, #7c6af7);
  }

  .icon.success {
    color: var(--success, #4ade80);
  }

  .icon.error {
    color: var(--danger, #f87171);
  }

  p {
    margin: 0;
    font-size: 0.9375rem;
    color: var(--text-2, #9aa0b0);
    text-align: center;
    max-width: 320px;
  }

  .error-msg {
    color: var(--danger, #f87171);
  }

  .hint {
    font-size: 0.8125rem;
    color: var(--text-3, #5c6070);
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }
</style>
