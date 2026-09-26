<script lang="ts">
  import type { PageData, ActionData } from './$types';
  export let data: PageData;
  export let form: ActionData;
</script>

<main>
  <header>
    <h1>API Keys</h1>
    <a href="/">Back to overview</a>
  </header>

  <section aria-labelledby="create-heading">
    <h2 id="create-heading">Create key</h2>
    <form method="POST" action="?/create">
      <label>
        Name
        <input type="text" name="name" required placeholder="e.g. ci-runner" />
      </label>
      <label>
        Role
        <select name="role">
          <option value="viewer">viewer</option>
          <option value="operator">operator</option>
          <option value="admin">admin</option>
        </select>
      </label>
      <button type="submit">Create</button>
    </form>

    {#if form?.error}
      <p class="error">{form.error}</p>
    {/if}

    {#if form?.created}
      <div class="created-key" role="alert">
        <p>Key created. Copy it now — it will not be shown again.</p>
        <code>{form.created.key}</code>
        <button
          type="button"
          onclick={() => navigator.clipboard.writeText(form?.created?.key ?? '')}
        >Copy</button>
      </div>
    {/if}
  </section>

  <section aria-labelledby="keys-heading">
    <h2 id="keys-heading">Keys ({data.keys.length})</h2>
    {#if data.keys.length === 0}
      <p>No keys yet.</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th scope="col">Name</th>
            <th scope="col">Role</th>
            <th scope="col">Created</th>
            <th scope="col">Last used</th>
            <th scope="col"></th>
          </tr>
        </thead>
        <tbody>
          {#each data.keys as k (k.id)}
            <tr>
              <td>{k.name}</td>
              <td>{k.role}</td>
              <td>{new Date(k.created_at).toLocaleString()}</td>
              <td>{k.last_used_at ? new Date(k.last_used_at).toLocaleString() : 'Never'}</td>
              <td>
                <form method="POST" action="?/revoke">
                  <input type="hidden" name="id" value={k.id} />
                  <button type="submit" class="danger">Revoke</button>
                </form>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>

  <section aria-labelledby="claude-heading">
    <h2 id="claude-heading">Configuring Claude Code</h2>
    <p>
      Set <code>ANTHROPIC_BASE_URL=http://localhost:8080</code> and
      <code>ANTHROPIC_API_KEY=&lt;your-key&gt;</code> in your Claude Code config.
    </p>
  </section>
</main>

<style>
  main { max-width: 900px; margin: 2rem auto; font-family: system-ui, sans-serif; padding: 0 1rem; }
  header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 2rem; }
  h1 { margin: 0; font-size: 1.5rem; }
  h2 { margin-top: 0; font-size: 1.1rem; border-bottom: 1px solid #eee; padding-bottom: 0.25rem; }
  section { margin-bottom: 2rem; }
  form { display: flex; gap: 0.75rem; align-items: flex-end; flex-wrap: wrap; }
  label { display: flex; flex-direction: column; gap: 0.25rem; font-size: 0.875rem; font-weight: 600; }
  input, select { padding: 0.4rem 0.5rem; border: 1px solid #ccc; border-radius: 4px; font-size: 0.875rem; }
  button { padding: 0.4rem 0.75rem; background: #1a1a2e; color: #fff; border: none; border-radius: 4px; cursor: pointer; font-size: 0.875rem; }
  button.danger { background: #c0392b; }
  table { width: 100%; border-collapse: collapse; font-size: 0.9rem; }
  th, td { text-align: left; padding: 0.4rem 0.6rem; border-bottom: 1px solid #eee; }
  th { background: #f5f5f5; font-weight: 600; }
  .error { color: #c0392b; font-size: 0.875rem; margin-top: 0.5rem; }
  .created-key { background: #f0fdf4; border: 1px solid #27ae60; border-radius: 4px; padding: 0.75rem 1rem; margin-top: 0.75rem; display: flex; flex-direction: column; gap: 0.5rem; }
  .created-key code { font-size: 0.85rem; word-break: break-all; background: #fff; padding: 0.25rem 0.5rem; border-radius: 3px; border: 1px solid #ccc; }
  a { color: #1a1a2e; font-size: 0.875rem; }
  code { background: #f5f5f5; padding: 0.1rem 0.3rem; border-radius: 3px; font-size: 0.85rem; }
</style>
