<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { api } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { Logo } from '$lib/components/index.js';

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

<div class="login-shell">
  <div class="login-card">
    <div class="brand">
      <Logo size={32} />
      <span class="brand-name">VKDG</span>
    </div>
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
        <button type="submit" disabled={loading}>
          {loading ? m.login_busy() : mode === 'set' ? m.login_set_submit() : m.login_submit()}
        </button>
      </form>
    {/if}
  </div>
</div>

<style>
  .hint {
    color: var(--text-2);
    font-size: 0.8125rem;
    margin: 0 0 12px;
  }

  .login-shell {
    min-height: 100vh;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--bg-base);
    padding: 24px;
  }

  .login-card {
    width: 100%;
    max-width: 380px;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    padding: 36px 32px;
    display: flex;
    flex-direction: column;
    gap: 24px;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .brand-name {
    font-size: 1.125rem;
    font-weight: 700;
    color: var(--text-1);
    letter-spacing: -0.02em;
  }

  h1 {
    font-size: 1.25rem;
    font-weight: 600;
    color: var(--text-1);
    margin: 0;
  }

  form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  label {
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--text-2);
  }

  input {
    width: 100%;
    background: var(--bg-base);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--text-1);
    font-size: 0.9375rem;
    padding: 0.5625rem 0.75rem;
    transition: border-color 0.15s;
    box-sizing: border-box;
  }

  input:focus {
    border-color: var(--accent);
    outline: none;
  }

  input:disabled {
    opacity: 0.5;
  }

  .error-msg {
    color: var(--danger);
    font-size: 0.8125rem;
    margin: 0;
  }

  button[type='submit'] {
    background: var(--accent);
    color: #fff;
    border: none;
    border-radius: var(--radius-sm);
    font-size: 0.9375rem;
    font-weight: 500;
    padding: 0.5625rem 1rem;
    cursor: pointer;
    transition: background 0.15s;
    margin-top: 4px;
  }

  button[type='submit']:hover:not(:disabled) {
    background: var(--accent-hover);
  }

  button[type='submit']:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }
</style>
