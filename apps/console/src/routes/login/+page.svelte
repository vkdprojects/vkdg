<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { api } from '$lib/api.js';
  import { Logo } from '$lib/components/index.js';

  let token = $state('');
  let loading = $state(false);
  let error = $state('');

  onMount(async () => {
    // Already logged in? Redirect to home.
    try {
      await api.me();
      goto('/');
    } catch {
      // not authenticated, stay on login
    }
  });

  async function handleSubmit(e: Event) {
    e.preventDefault();
    if (!token.trim()) return;
    loading = true;
    error = '';
    try {
      await api.login(token.trim());
      goto('/');
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
    <h1>Sign in</h1>
    <form onsubmit={handleSubmit}>
      <label for="token">Bootstrap token</label>
      <input
        id="token"
        type="password"
        name="token"
        required
        autocomplete="off"
        bind:value={token}
        disabled={loading}
        aria-describedby={error ? 'login-error' : undefined}
      />
      {#if error}
        <p id="login-error" class="error-msg" role="alert">{error}</p>
      {/if}
      <button type="submit" disabled={loading}>
        {loading ? 'Signing in…' : 'Sign in'}
      </button>
    </form>
  </div>
</div>

<style>
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
