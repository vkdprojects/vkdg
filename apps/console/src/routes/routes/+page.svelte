<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api.js';
  import type { RouteSummary, RoutePreview } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { Badge, Button, Card, EmptyState, Spinner } from '$lib/components/index.js';
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
  <div class="page-header">
    <h1 class="page-title">{m.nav_routes()} ({routes.length})</h1>
  </div>

  <Card>
    <h2 id="preview-heading">{m.route_preview_heading()}</h2>
    <form onsubmit={doPreview} class="preview-form">
      <label>
        {m.route_preview_model_label()}
        <input type="text" bind:value={previewModel} placeholder="e.g. claude-3-5-sonnet-20241022" />
      </label>
      <Button type="submit" disabled={previewing}>{m.route_preview_button()}</Button>
    </form>

    {#if previewError}
      <p class="error-msg" role="alert">{previewError}</p>
    {/if}

    {#if preview}
      <div class="preview-result">
        <p><strong>{m.route_preview_model_label()}</strong> <span class="mono">{preview.model}</span></p>
        <p><strong>{m.route_eligible_connections()}</strong></p>
        {#if preview.eligible_connections.length === 0}
          <p class="muted">{m.route_none()}</p>
        {:else}
          <ul>
            {#each preview.eligible_connections as id}
              <li class="mono">{id}</li>
            {/each}
          </ul>
        {/if}
        {#if preview.excluded_connections.length > 0}
          <p><strong>{m.route_excluded_connections()}</strong></p>
          <ul>
            {#each preview.excluded_connections as ex}
              <li><span class="mono">{ex.id}</span> — {ex.reason}</li>
            {/each}
          </ul>
        {/if}
      </div>
    {/if}
  </Card>

  <section aria-labelledby="routes-heading">
    <h2 id="routes-heading">{m.nav_routes()}</h2>
    {#if loading}
      <div class="loading"><Spinner size="sm" /> {m.common_loading()}</div>
    {:else if routes.length === 0}
      <EmptyState title={m.route_empty()} />
    {:else}
      <table>
        <thead>
          <tr>
            <th scope="col">{m.route_id()}</th>
            <th scope="col">{m.route_strategy()}</th>
            <th scope="col">{m.route_match_models()}</th>
            <th scope="col">{m.route_targets()}</th>
          </tr>
        </thead>
        <tbody>
          {#each routes as r (r.id)}
            <tr>
              <td class="mono">{r.id}</td>
              <td><Badge status={r.strategy} /></td>
              <td class="mono">{r.match_models.join(', ') || m.common_any()}</td>
              <td class="mono">{r.targets.join(', ') || m.common_none()}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>
</div>

<style>
  .page-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 1.5rem;
  }

  .page-title {
    margin: 0;
  }

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
    min-width: 280px;
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
