<script lang="ts">
  import type { PageData, ActionData } from './$types';
  export let data: PageData;
  export let form: ActionData;
</script>

<main>
  <header>
    <h1>Routes</h1>
    <a href="/">Back to overview</a>
  </header>

  <section aria-labelledby="preview-heading">
    <h2 id="preview-heading">Preview routing for model</h2>
    <form method="POST" action="?/preview">
      <label>
        Model name
        <input type="text" name="model" placeholder="e.g. claude-3-5-sonnet-20241022" />
      </label>
      <button type="submit">Preview</button>
    </form>

    {#if form?.error}
      <p class="error">{form.error}</p>
    {/if}

    {#if form?.preview}
      <div class="preview-result">
        <p><strong>Model:</strong> {form.preview.model}</p>
        <p><strong>Eligible connections:</strong></p>
        {#if form.preview.eligible_connections.length === 0}
          <p class="muted">None</p>
        {:else}
          <ul>
            {#each form.preview.eligible_connections as id}
              <li>{id}</li>
            {/each}
          </ul>
        {/if}
        {#if form.preview.excluded_connections.length > 0}
          <p><strong>Excluded:</strong></p>
          <ul>
            {#each form.preview.excluded_connections as ex}
              <li>{ex.id} — {ex.reason}</li>
            {/each}
          </ul>
        {/if}
      </div>
    {/if}
  </section>

  <section aria-labelledby="routes-heading">
    <h2 id="routes-heading">Routes ({data.routes.length})</h2>
    {#if data.routes.length === 0}
      <p>No routes configured.</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th scope="col">ID</th>
            <th scope="col">Strategy</th>
            <th scope="col">Match models</th>
            <th scope="col">Targets</th>
          </tr>
        </thead>
        <tbody>
          {#each data.routes as r (r.id)}
            <tr>
              <td>{r.id}</td>
              <td>{r.strategy}</td>
              <td>{r.match_models.join(', ') || '*'}</td>
              <td>{r.targets.join(', ')}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
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
  input { padding: 0.4rem 0.5rem; border: 1px solid #ccc; border-radius: 4px; font-size: 0.875rem; min-width: 280px; }
  button { padding: 0.4rem 0.75rem; background: #1a1a2e; color: #fff; border: none; border-radius: 4px; cursor: pointer; font-size: 0.875rem; }
  table { width: 100%; border-collapse: collapse; font-size: 0.9rem; }
  th, td { text-align: left; padding: 0.4rem 0.6rem; border-bottom: 1px solid #eee; }
  th { background: #f5f5f5; font-weight: 600; }
  .error { color: #c0392b; font-size: 0.875rem; margin-top: 0.5rem; }
  .muted { color: #888; font-size: 0.875rem; }
  .preview-result { margin-top: 0.75rem; background: #f9f9f9; border: 1px solid #ddd; border-radius: 4px; padding: 0.75rem 1rem; font-size: 0.875rem; }
  .preview-result ul { margin: 0.25rem 0 0.75rem 1.25rem; padding: 0; }
  a { color: #1a1a2e; font-size: 0.875rem; }
</style>
