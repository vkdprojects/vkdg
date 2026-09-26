<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api.js';
  import type { RouteSummary, RoutePreview } from '$lib/api.js';
  import { EmptyState, Spinner } from '$lib/components/index.js';
  import { toast } from 'svelte-sonner';

  let routes = $state<RouteSummary[]>([]);
  let loading = $state(true);
  let previewModel = $state('');
  let previewing = $state(false);
  let previewError = $state('');
  let preview = $state<RoutePreview | null>(null);

  onMount(async () => {
    try {
      const res = await api.listRoutes();
      routes = res.items;
    } catch (e) {
      toast.error((e as Error).message);
    } finally {
      loading = false;
    }
  });

  async function doPreview(e: Event) {
    e.preventDefault();
    if (!previewModel.trim()) { previewError = 'Model is required'; return; }
    previewing = true;
    previewError = '';
    preview = null;
    try {
      preview = await api.previewRoute(previewModel.trim());
    } catch (err) {
      previewError = (err as Error).message;
    } finally {
      previewing = false;
    }
  }
</script>

<div class="page">
  <h1>Routes ({routes.length})</h1>

  <section aria-labelledby="preview-heading">
    <h2 id="preview-heading">Preview routing for model</h2>
    <form onsubmit={doPreview} class="preview-form">
      <label>
        Model name
        <input type="text" bind:value={previewModel} placeholder="e.g. claude-3-5-sonnet-20241022" />
      </label>
      <button type="submit" disabled={previewing}>Preview</button>
    </form>

    {#if previewError}
      <p class="error-msg" role="alert">{previewError}</p>
    {/if}

    {#if preview}
      <div class="preview-result">
        <p><strong>Model:</strong> {preview.model}</p>
        <p><strong>Eligible connections:</strong></p>
        {#if preview.eligible_connections.length === 0}
          <p class="muted">None</p>
        {:else}
          <ul>
            {#each preview.eligible_connections as id}
              <li>{id}</li>
            {/each}
          </ul>
        {/if}
        {#if preview.excluded_connections.length > 0}
          <p><strong>Excluded:</strong></p>
          <ul>
            {#each preview.excluded_connections as ex}
              <li>{ex.id} — {ex.reason}</li>
            {/each}
          </ul>
        {/if}
      </div>
    {/if}
  </section>

  <section aria-labelledby="routes-heading">
    <h2 id="routes-heading">All routes</h2>
    {#if loading}
      <div class="loading"><Spinner size="sm" /> Loading…</div>
    {:else if routes.length === 0}
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
          {#each routes as r (r.id)}
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
  .loading {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-3);
    font-size: 0.875rem;
    padding: 16px 0;
  }

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

  .preview-form button:hover:not(:disabled) {
    background: var(--accent-hover);
  }

  .preview-form button:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .error-msg {
    margin-top: 8px;
    font-size: 0.8125rem;
    color: var(--danger);
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
