<script lang="ts">
  import { api } from '$lib/api.js';
  import type { RequestStatusFilter, RequestSummary } from '$lib/api.js';
  import { Button, EmptyState, Spinner } from '$lib/components/index.js';
  import { poll } from '$lib/live.svelte.js';
  import { m } from '$lib/paraglide/messages.js';
  import { PauseIcon, PlayIcon, SearchIcon } from 'lucide-svelte';
  import { toast } from 'svelte-sonner';
  import RequestDrawer from './RequestDrawer.svelte';
  import RequestsTable from './RequestsTable.svelte';

  let requests = $state<RequestSummary[]>([]);
  let loading = $state(true);
  let paused = $state(false);
  let statusFilter = $state<RequestStatusFilter>('all');

  let searchId = $state('');
  let searchError = $state('');
  let detailOpen = $state(false);
  let detailLoading = $state(false);
  let detailError = $state('');
  let detail = $state<RequestSummary | null>(null);

  const filterOptions = $derived([
    { value: 'all', label: m.request_filter_all() },
    { value: 'completed', label: m.request_status_completed() },
    { value: 'cancelled', label: m.request_status_cancelled() },
    { value: 'failed', label: m.request_status_failed() },
  ]);

  async function load() {
    try {
      requests = (await api.listRequests(50, statusFilter)).items;
    } catch (e) {
      toast.error((e as Error).message);
    } finally {
      loading = false;
    }
  }

  // Live every 3s while the tab is visible; pausing only gates the tick, the
  // first load and filter changes still fetch.
  poll(() => (paused ? Promise.resolve() : load()), 3000);

  function onFilterChange(v: string) {
    statusFilter = v as RequestStatusFilter;
    loading = true;
    load();
  }

  async function openDetail(id: string) {
    detailOpen = true;
    detailLoading = true;
    detailError = '';
    detail = null;
    try {
      detail = await api.getRequest(id);
    } catch (err) {
      detailError = (err as Error).message;
    } finally {
      detailLoading = false;
    }
  }

  function search(e: Event) {
    e.preventDefault();
    if (!searchId.trim()) { searchError = m.request_id_required(); return; }
    searchError = '';
    openDetail(searchId.trim());
  }
</script>

<div class="page">
  <div class="page-header">
    <div>
      <h1>{m.nav_requests()}</h1>
      <p class="refresh-note" aria-live="polite">
        <span class="live-dot" class:paused></span>
        {paused ? m.request_paused() : m.request_live()}
      </p>
    </div>
    <div class="page-actions">
      <Button
        variant="outline"
        size="md"
        onclick={() => (paused = !paused)}
        ariaLabel={paused ? m.request_resume_label() : m.request_pause_label()}
      >
        {#if paused}<PlayIcon size={14} aria-hidden="true" />{m.request_resume()}{:else}<PauseIcon size={14} aria-hidden="true" />{m.request_pause()}{/if}
      </Button>
    </div>
  </div>

  <div class="toolbar">
    <div class="segmented" role="group" aria-label={m.request_filter()}>
      {#each filterOptions as opt (opt.value)}
        <button
          type="button"
          aria-pressed={statusFilter === opt.value}
          onclick={() => onFilterChange(opt.value)}
        >
          {opt.label}
        </button>
      {/each}
    </div>

    <form onsubmit={search} class="search-form" aria-labelledby="search-heading">
      <h2 id="search-heading" class="sr-only">{m.request_search()}</h2>
      <label class="search-field">
        <span class="sr-only">{m.request_id_label()}</span>
        <span class="search-icon" aria-hidden="true"><SearchIcon size={14} /></span>
        <input type="text" bind:value={searchId} placeholder="req_..." aria-label={m.request_id_label()} />
      </label>
      <Button type="submit" size="md">{m.request_search_button()}</Button>
    </form>
  </div>
  {#if searchError}
    <p class="error-msg" role="alert">{searchError}</p>
  {/if}

  <section class="panel" aria-labelledby="recent-heading">
    <div class="panel-head">
      <h2 id="recent-heading">{m.request_recent()}</h2>
      <span class="count mono">{requests.length}</span>
    </div>
    {#if loading}
      <div class="panel-body">
        <div class="loading"><Spinner size="sm" /> {m.common_loading()}</div>
      </div>
    {:else if requests.length === 0}
      <div class="panel-body">
        <EmptyState title={m.request_empty()} description={m.request_empty_desc()} />
      </div>
    {:else}
      <RequestsTable {requests} onopen={openDetail} />
    {/if}
  </section>
</div>

<RequestDrawer bind:open={detailOpen} loading={detailLoading} error={detailError} {detail} />

<style>
  .refresh-note {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-3);
    margin: var(--space-2) 0 0;
  }

  .live-dot {
    width: var(--dot-size);
    height: var(--dot-size);
    border-radius: var(--radius-full);
    background: var(--success);
    box-shadow: 0 0 0 var(--space-1) color-mix(in oklch, var(--success) 25%, transparent);
    animation: pulse var(--dur-pulse) ease-in-out infinite;
  }

  .live-dot.paused {
    background: var(--text-3);
    box-shadow: none;
    animation: none;
  }

  @keyframes pulse {
    50% { box-shadow: 0 0 0 var(--space-2) color-mix(in oklch, var(--success) 8%, transparent); }
  }

  @media (prefers-reduced-motion: reduce) {
    .live-dot { animation: none; }
  }

  /* Toolbar: segmented status filter + id search */
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    margin-bottom: var(--space-4);
  }

  .search-form {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex: 1 1 var(--dialog-w);
    justify-content: flex-end;
    min-width: 0;
  }

  .search-field {
    position: relative;
    flex: 1 1 auto;
    max-width: var(--dialog-w);
    min-width: 0;
  }

  .search-field input {
    padding-left: var(--space-6);
    min-height: var(--control-h-sm);
    border-radius: var(--radius-full);
    font-family: var(--font-mono);
    font-size: var(--text-sm);
  }

  .search-icon {
    position: absolute;
    left: var(--space-3);
    top: 50%;
    transform: translateY(-50%);
    color: var(--text-3);
    display: flex;
    pointer-events: none;
  }

  .search-form :global(.btn) { min-height: var(--control-h-sm); }

  @media (max-width: 640px) {
    .toolbar { flex-direction: column; align-items: stretch; }
    .search-form { flex: 1 1 auto; justify-content: stretch; }
    .search-field { max-width: none; }
  }

  .count {
    font-size: var(--text-xs);
    color: var(--text-2);
    background: var(--bg-inset);
    border: var(--border-w) solid var(--border);
    border-radius: var(--radius-full);
    padding: var(--space-0) var(--space-3);
  }

  .loading {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-3);
    font-size: var(--text-sm);
    padding: var(--space-3) 0;
  }
</style>
