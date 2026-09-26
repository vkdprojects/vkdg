<script lang="ts">
  import type { PageData, ActionData } from './$types';
  import { Badge, EmptyState } from '$lib/components/index.js';

  let { data, form }: { data: PageData; form: ActionData } = $props();

  function fmtDuration(ms: number | null): string {
    if (ms === null) return 'pending';
    if (ms < 1000) return `${ms}ms`;
    return `${(ms / 1000).toFixed(2)}s`;
  }
</script>

<div class="page">
  <h1>Requests</h1>

  <section aria-labelledby="search-heading">
    <h2 id="search-heading">Look up request</h2>
    <form method="POST" action="?/search" class="search-form">
      <label>
        Request ID
        <input type="text" name="id" placeholder="req_..." />
      </label>
      <button type="submit">Search</button>
    </form>

    {#if form?.error}
      <p class="error-msg" role="alert">{form.error}</p>
    {/if}

    {#if form?.detail}
      {@const d = form.detail}
      <div class="detail-card">
        <dl class="info-grid">
          <dt>Request ID</dt><dd class="mono">{d.request_id}</dd>
          <dt>Model</dt><dd>{d.model}</dd>
          <dt>API type</dt><dd>{d.api_type}</dd>
          <dt>Status</dt><dd><Badge status={d.status} /></dd>
          <dt>Connection</dt><dd>{d.connection_id ?? '—'}</dd>
          <dt>Started</dt><dd>{new Date(d.started_at_ms).toLocaleString()}</dd>
          <dt>Duration</dt><dd>{fmtDuration(d.duration_ms)}</dd>
        </dl>
      </div>
    {/if}
  </section>

  <section aria-labelledby="recent-heading">
    <h2 id="recent-heading">Recent requests ({data.requests.length})</h2>
    {#if data.requests.length === 0}
      <EmptyState title="No requests recorded yet." description="Requests will appear here once your gateway receives traffic." />
    {:else}
      <table>
        <thead>
          <tr>
            <th scope="col">Request ID</th>
            <th scope="col">Model</th>
            <th scope="col">Status</th>
            <th scope="col">Duration</th>
            <th scope="col">Started</th>
          </tr>
        </thead>
        <tbody>
          {#each data.requests as r (r.request_id)}
            <tr>
              <td class="mono">{r.request_id}</td>
              <td>{r.model}</td>
              <td><Badge status={r.status} /></td>
              <td>{fmtDuration(r.duration_ms)}</td>
              <td>{new Date(r.started_at_ms).toLocaleString()}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>
</div>

<style>
  .search-form {
    display: flex;
    gap: 0.75rem;
    align-items: flex-end;
    flex-wrap: wrap;
  }

  .search-form label {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--text-2);
  }

  .search-form input {
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--text-1);
    font-size: 0.875rem;
    padding: 0.4375rem 0.625rem;
    min-width: 280px;
    transition: border-color 0.15s;
  }

  .search-form input:focus {
    border-color: var(--accent);
    outline: none;
  }

  .search-form input::placeholder {
    color: var(--text-3);
  }

  .search-form button {
    padding: 0.4375rem 0.875rem;
    background: var(--accent);
    color: #fff;
    border: none;
    border-radius: var(--radius-sm);
    cursor: pointer;
    font-size: 0.8125rem;
    font-weight: 500;
    transition: background 0.1s;
    white-space: nowrap;
  }

  .search-form button:hover {
    background: var(--accent-hover);
  }

  .detail-card {
    margin-top: 0.75rem;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 0.875rem 1rem;
  }

  .info-grid {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 0.375rem 1.25rem;
    font-size: 0.875rem;
    margin: 0;
  }

  dt {
    color: var(--text-3);
    font-weight: 500;
  }

  dd {
    margin: 0;
    color: var(--text-2);
  }
</style>
