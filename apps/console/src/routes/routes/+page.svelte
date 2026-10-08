<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api.js';
  import type { RouteSummary, RoutePreview } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { Badge, Button, EmptyState } from '$lib/components/index.js';
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
    <div>
      <h1>{m.nav_routes()} <span class="count mono">{routes.length}</span></h1>
    </div>
  </div>

  <section class="panel" aria-labelledby="preview-heading">
    <div class="panel-head">
      <h2 id="preview-heading">{m.route_preview_heading()}</h2>
    </div>
    <div class="panel-body">
      <form onsubmit={doPreview} class="preview-form">
        <label class="preview-field">
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
    </div>
  </section>

  <section class="panel" aria-labelledby="routes-heading">
    <div class="panel-head">
      <h2 id="routes-heading">{m.nav_routes()}</h2>
    </div>
    {#if loading}
      <div class="panel-body loading" aria-busy="true">
        <div class="skeleton row"></div>
        <div class="skeleton row"></div>
        <div class="skeleton row"></div>
      </div>
    {:else if routes.length === 0}
      <div class="panel-body">
        <EmptyState title={m.route_empty()} />
      </div>
    {:else}
      <div class="table-wrap">
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
                <td class="mono id">{r.id}</td>
                <td><Badge status={r.strategy} /></td>
                <td class="mono">{r.match_models.join(', ') || m.common_any()}</td>
                <td class="mono">{r.targets.join(', ') || m.common_none()}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  </section>
</div>

<style>
  .count {
    color: var(--text-3);
    font-weight: var(--weight-regular);
    font-size: var(--text-md);
    margin-left: var(--space-1);
  }

  .loading {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .skeleton.row {
    height: var(--control-h);
  }

  .preview-form {
    display: flex;
    gap: var(--space-3);
    align-items: flex-end;
    flex-wrap: wrap;
  }

  .preview-field {
    flex: 1 1 var(--col-lg);
    min-width: 0;
  }

  .preview-form :global(.btn) {
    min-height: var(--control-h);
  }

  @media (max-width: 480px) {
    .preview-form :global(.btn) {
      width: 100%;
    }
  }

  .preview-result {
    margin-top: var(--space-4);
    background: var(--bg-inset);
    border: var(--border-w) solid var(--border);
    border-radius: var(--radius);
    padding: var(--space-4);
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
  }

  .preview-result p {
    margin: 0 0 var(--space-2);
  }

  .preview-result p strong {
    color: var(--text-1);
    font-weight: var(--weight-semibold);
  }

  .preview-result ul {
    margin: var(--space-1) 0 var(--space-3) var(--space-5);
    padding: 0;
    color: var(--text-2);
  }

  .id {
    color: var(--text-1);
  }
</style>
