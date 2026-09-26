<script lang="ts">
  import type { ActionData } from './$types';
  import { Logo } from '$lib/components/index.js';

  let { form }: { form: ActionData } = $props();
</script>

<div class="login-shell">
  <div class="login-card">
    <div class="brand">
      <Logo size={32} />
      <span class="brand-name">VKDG</span>
    </div>
    <h1>Sign in</h1>
    <form method="POST">
      <label for="token">Bootstrap token</label>
      <input
        id="token"
        type="password"
        name="token"
        required
        autocomplete="off"
        aria-describedby={form?.error ? 'login-error' : undefined}
      />
      {#if form?.error}
        <p id="login-error" class="error-msg" role="alert">{form.error}</p>
      {/if}
      <button type="submit">Sign in</button>
    </form>
  </div>
</div>

<style>
  .login-shell {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 100vh;
    background: var(--bg-base);
  }

  .login-card {
    width: 100%;
    max-width: 360px;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 2rem;
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    font-weight: 700;
    font-size: 15px;
    color: var(--text-1);
    letter-spacing: 0.04em;
  }


  h1 {
    font-size: 1.125rem;
    font-weight: 600;
    margin: 0;
    color: var(--text-1);
  }

  form {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  label {
    display: block;
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--text-2);
    margin-bottom: 0.25rem;
  }

  input {
    display: block;
    width: 100%;
    padding: 0.4375rem 0.625rem;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--text-1);
    font-size: 0.875rem;
    transition: border-color 0.15s;
    box-sizing: border-box;
  }

  input:focus {
    border-color: var(--accent);
    outline: none;
  }

  button {
    width: 100%;
    padding: 0.5625rem;
    background: var(--accent);
    color: #fff;
    border: none;
    border-radius: var(--radius-sm);
    cursor: pointer;
    font-size: 0.875rem;
    font-weight: 500;
    transition: background 0.1s;
  }

  button:hover {
    background: var(--accent-hover);
  }

  .error-msg {
    color: var(--danger);
    font-size: 0.8125rem;
    margin: 0;
  }
</style>
