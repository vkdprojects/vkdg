<script lang="ts">
  import type { PageData, ActionData } from './$types';
  import { EmptyState } from '$lib/components/index.js';

  let { data, form }: { data: PageData; form: ActionData } = $props();
</script>

<div class="page">
  <h1>Routes ({data.routes.length})</h1>

  <section aria-labelledby="preview-heading">
    <h2 id="preview-heading">Preview routing for model</h2>
    <form method="POST" action="?/preview" class="preview-form">
      <label>
        Model name
        <input type="text" name="model" placeholder="e.g. claude-3-5-sonnet-20241022" />
      </label>
      <button type="submit">Preview</button>
    </form>

    {#if form?.error}
      <p class="error-msg" role="alert">{form.error}</p>
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
    <h2 id="routes-heading">All routes</h2>
    {#if data.routes.length === 0}
      <EmptyState title="No routes configured." description="Routes define how models are matched to connection combos." />
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
</div>

<style>
  .preview-form {
    display: flex;
    gap: 0.75rem;
    align-items: flex-end;
    flex-wrap: wrap;
  }

  .preview-form label {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--text-2);
  }

  .preview-form input {
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--text-1);
    font-size: 0.875rem;
    padding: 0.4375rem 0.625rem;
    min-width: 280px;
    transition: border-color 0.15s;
  }

  .preview-form input:focus {
    border-color: var(--accent);
    outline: none;
  }

  .preview-form input::placeholder {
    color: var(--text-3);
  }

  .preview-form button {
    padding: 0.4375rem 0.875rem;
    background: var(--accent);
    color: #fff;
    border: none;
    border-radius: var(--radius-sm);
    cursor: pointer;
    font-size: 0.8125rem;
    font-weight: 500;
    transition: background 0.1s;
  }

  .preview-form button:hover {
    background: var(--accent-hover);
  }

  .preview-result {
    margin-top: 0.875rem;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 0.875rem 1rem;
    font-size: 0.875rem;
  }

  .preview-result p {
    margin: 0 0 0.375rem;
    color: var(--text-2);
  }

  .preview-result ul {
    margin: 0.25rem 0 0.625rem 1.25rem;
    padding: 0;
    color: var(--text-2);
    font-size: 0.875rem;
  }

  .muted {
    color: var(--text-3);
  }
</style>
