<script lang="ts">
  import type { ActionData } from './$types';
  export let form: ActionData;
</script>

<main>
  <h1>VKDG Console</h1>
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
      <p id="login-error" class="error" role="alert">{form.error}</p>
    {/if}
    <button type="submit">Sign in</button>
  </form>
</main>

<style>
  main {
    max-width: 360px;
    margin: 8rem auto;
    font-family: system-ui, sans-serif;
  }
  label {
    display: block;
    margin-bottom: 0.25rem;
    font-size: 0.875rem;
  }
  input {
    display: block;
    width: 100%;
    padding: 0.5rem;
    margin-bottom: 0.75rem;
    border: 1px solid #ccc;
    border-radius: 4px;
    box-sizing: border-box;
  }
  button {
    width: 100%;
    padding: 0.5rem;
    background: #1a1a2e;
    color: #fff;
    border: none;
    border-radius: 4px;
    cursor: pointer;
  }
  .error {
    color: #c0392b;
    font-size: 0.875rem;
    margin-bottom: 0.5rem;
  }
</style>
