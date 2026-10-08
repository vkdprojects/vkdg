<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { api } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { Button, Spinner } from '$lib/components/index.js';

  // `token`: first run, sign in with the bootstrap token.
  // `password`: a console password exists; the token no longer works.
  // `set`: signed in with the token; choose the password before continuing,
  // so a sign-out never needs a gateway restart to get back in.
  type Mode = 'loading' | 'token' | 'password' | 'set';
  let mode = $state<Mode>('loading');
  let secret = $state('');
  let confirmation = $state('');
  let loading = $state(false);
  let error = $state('');

  onMount(async () => {
    try {
      await api.me();
      goto('/');
      return;
    } catch {
      // not signed in
    }
    try {
      mode = (await api.setupStatus()).password_set ? 'password' : 'token';
    } catch {
      mode = 'token';
    }
  });

  async function submit(e: Event) {
    e.preventDefault();
    if (!secret) return;
    loading = true;
    error = '';
    try {
      if (mode === 'set') {
        if (secret !== confirmation) {
          error = m.login_password_mismatch();
          return;
        }
        await api.setPassword(secret);
        goto('/');
      } else if (mode === 'password') {
        await api.login({ password: secret });
        goto('/');
      } else {
        await api.login({ token: secret.trim() });
        mode = 'set';
        secret = '';
      }
    } catch (err) {
      error = (err as Error).message;
    } finally {
      loading = false;
    }
  }
</script>

<div class="login-shell" data-theme="dark">
  <div class="scene" aria-hidden="true">
    <img src="/brand/hero.jpg" alt="" class="hero" fetchpriority="high" />
    <span class="dots"></span>
  </div>

  <main class="login-card">
    <div class="card-body">
      <h1>{mode === 'set' ? m.login_set_heading() : m.login_submit()}</h1>
      {#if mode === 'set'}
        <p class="hint">{m.login_set_hint()}</p>
      {/if}
      {#if mode !== 'loading'}
        <form onsubmit={submit}>
          <label for="secret">
            {mode === 'token' ? m.login_token_label() : mode === 'set' ? m.login_new_password_label() : m.login_password_label()}
          </label>
          <input
            id="secret"
            type="password"
            name={mode === 'token' ? 'token' : 'password'}
            required
            autocomplete={mode === 'password' ? 'current-password' : mode === 'set' ? 'new-password' : 'off'}
            bind:value={secret}
            disabled={loading}
            aria-describedby={error ? 'login-error' : undefined}
          />
          {#if mode === 'set'}
            <label for="confirm">{m.login_confirm_password_label()}</label>
            <input
              id="confirm"
              type="password"
              name="confirm"
              required
              autocomplete="new-password"
              bind:value={confirmation}
              disabled={loading}
            />
          {/if}
          {#if error}
            <p id="login-error" class="error-msg" role="alert">{error}</p>
          {/if}
          <Button type="submit" size="lg" disabled={loading}>
            {loading ? m.login_busy() : mode === 'set' ? m.login_set_submit() : m.login_submit()}
          </Button>
        </form>
      {:else}
        <div class="loading-row"><Spinner size="sm" /> {m.common_loading()}</div>
      {/if}
    </div>
  </main>
</div>

<style>
  /* The scene is always dark: data-theme="dark" on .login-shell makes
     tokens.css recompute the whole palette for this subtree. */
  .login-shell {
    position: relative;
    isolation: isolate;
    min-height: 100vh;
    min-height: 100dvh;
    display: grid;
    place-items: center;
    padding: var(--space-5) var(--space-4);
    background: var(--bg-base);
    color: var(--text-1);
    overflow: hidden;
  }

  /* Scene: glass fox render (palette = logo ink/teal/mint) + dot field.
     The card floats over the fox's shadow side; one slow breath of motion. */
  .scene { position: absolute; inset: 0; z-index: var(--z-behind); pointer-events: none; background: var(--bg-surface); }
  .hero {
    position: absolute; inset: 0; width: 100%; height: 100%;
    object-fit: cover; object-position: 50% 45%;
    animation: breathe var(--dur-ambient) ease-in-out infinite alternate;
  }
  .dots {
    position: absolute; inset: 0;
    background:
      radial-gradient(ellipse at 50% 55%, transparent 30%, color-mix(in oklch, var(--bg-base) 85%, transparent) 95%),
      var(--dot-field);
  }
  @keyframes breathe { to { transform: scale(1.04); } }
  @media (prefers-reduced-motion: reduce) { .hero { animation: none; } }
  @media (max-width: 640px) { .hero { object-position: 50% 30%; } }
  @media (min-width: 1000px) {
    .login-shell { justify-items: end; padding-right: clamp(var(--space-6), 9vw, calc(var(--space-8) * 2.5)); }
    .hero {
      inset: 0 auto 0 -12%; width: 82%; object-position: 50% 50%;
      /* fade the render's own right edge so there is no seam */
      mask-image: linear-gradient(90deg, var(--bg-base) 60%, transparent 98%);
      -webkit-mask-image: linear-gradient(90deg, var(--bg-base) 60%, transparent 98%);
    }
  }

  /* ── card ────────────────────────────────────────────────────────────── */
  .login-card {
    width: 100%;
    max-width: calc(var(--space-8) * 6.5);
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    animation: card-in var(--dur-3) var(--ease-out) both;
  }

  @keyframes card-in {
    from { opacity: 0; transform: translateY(var(--space-3)) scale(0.985); }
  }

  .card-body {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-6) var(--space-5);
    background: var(--glass-bg);
    -webkit-backdrop-filter: var(--glass-blur);
    backdrop-filter: var(--glass-blur);
    border: var(--border-w) solid var(--glass-border);
    border-radius: var(--radius-xl);
    box-shadow: var(--glass-shadow);
  }

  h1 {
    font-size: var(--text-xl);
    margin: 0;
    text-align: center;
  }

  .hint {
    color: var(--text-2);
    font-size: var(--text-sm);
    text-align: center;
    margin: calc(var(--space-2) * -1) 0 0;
  }

  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  input {
    min-height: var(--control-h-lg);
  }

  .error-msg {
    margin: 0;
    padding: var(--space-2) var(--space-3);
    background: var(--danger-subtle);
    border: var(--border-w) solid color-mix(in oklch, var(--danger) 30%, transparent);
    border-radius: var(--radius);
  }

  .loading-row {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    color: var(--text-2);
    font-size: var(--text-sm);
    padding: var(--space-3) 0;
  }

  :global(form button[type='submit']) {
    margin-top: var(--space-1);
    width: 100%;
  }

  @media (max-width: 999px) {
    /* Fox owns the top; card docks to the bottom like a sheet. */
    .login-shell { align-items: end; padding-bottom: max(var(--space-5), env(safe-area-inset-bottom)); }
    .hero {
      height: 62%; object-position: 50% 40%;
      /* melt the render's bottom edge into the background: no horizontal seam */
      mask-image: linear-gradient(180deg, var(--bg-base) 55%, transparent 100%);
      -webkit-mask-image: linear-gradient(180deg, var(--bg-base) 55%, transparent 100%);
    }
    .dots {
      background:
        linear-gradient(180deg, transparent 35%, var(--bg-base) 62%),
        var(--dot-field);
    }
  }
  @media (max-width: 400px) {
    .card-body { padding: var(--space-5) var(--space-4); }
  }
</style>
