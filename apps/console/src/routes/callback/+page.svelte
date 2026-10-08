<script lang="ts">
  import { onMount } from 'svelte';
  import { Check, X } from 'lucide-svelte';
  import { Spinner } from '$lib/components/index.js';

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
  <div class="callback-card" role="status" aria-live="polite">
    {#if status === 'pending'}
      <div class="icon pending"><Spinner size="lg" /></div>
      <p class="msg">{message}</p>
    {:else if status === 'done'}
      <div class="icon success"><Check size={28} strokeWidth={2.5} aria-hidden="true" /></div>
      <p class="msg">{message}</p>
    {:else}
      <div class="icon error"><X size={28} strokeWidth={2.5} aria-hidden="true" /></div>
      <p class="msg error-msg">{message}</p>
      <p class="hint">Close this window and try again.</p>
    {/if}
  </div>
</div>

<style>
  .callback-page {
    display: grid;
    place-items: center;
    min-height: 100vh;
    min-height: 100dvh;
    padding: var(--space-4);
    background:
      radial-gradient(ellipse 60% 50% at 50% 0%, color-mix(in oklch, var(--accent) 14%, transparent), transparent 70%),
      var(--bg-base);
    color: var(--text-1);
    font-family: var(--font-sans);
  }

  .callback-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    max-width: calc(var(--space-8) * 5.5);
    padding: var(--space-6) var(--space-5);
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-2);
    text-align: center;
    animation: card-in var(--dur-3) var(--ease-out) both;
  }

  @keyframes card-in { from { opacity: 0; transform: translateY(var(--space-2)); } }

  .icon {
    display: grid;
    place-items: center;
    width: var(--space-8);
    height: var(--space-8);
    border-radius: var(--radius-full);
    margin-bottom: var(--space-1);
  }

  .icon.pending { background: var(--accent-subtle); }

  .icon.success {
    background: var(--success-subtle);
    color: var(--success);
    animation: pop-in var(--dur-3) var(--ease-spring);
  }

  .icon.error {
    background: var(--danger-subtle);
    color: var(--danger);
    animation: pop-in var(--dur-3) var(--ease-spring);
  }

  @keyframes pop-in { from { opacity: 0; transform: scale(0.5); } }

  p { margin: 0; }

  .msg {
    font-size: var(--text-md);
    color: var(--text-1);
    overflow-wrap: anywhere;
  }

  .error-msg { color: var(--danger); margin-top: 0; }

  .hint {
    font-size: var(--text-sm);
    color: var(--text-3);
  }
</style>
