<script lang="ts">
  import type { PageData, ActionData } from './$types';
  export let data: PageData;
  export let form: ActionData;

  function fmtDuration(ms: number | null): string {
    if (ms === null) return 'pending';
    if (ms < 1000) return `${ms}ms`;
    return `${(ms / 1000).toFixed(2)}s`;
  }

  const statusColor: Record<string, string> = {
    success: '#27ae60',
    error: '#c0392b',
    pending: '#e67e22',
  };
</script>

<main>
  <header>
    <h1>Requests</h1>
    <a href="/">Back to overview</a>
  </header>

  <section aria-labelledby="search-heading">
    <h2 id="search-heading">Look up request</h2>
    <form method="POST" action="?/search">
      <label>
        Request ID
        <input type="text" name="id" placeholder="req_..." />
      </label>
      <button type="submit">Search</button>
    </form>

    {#if form?.error}
      <p class="error">{form.error}</p>
    {/if}

    {#if form?.detail}
      {@const d = form.detail}
      <div class="detail">
        <dl>
          <dt>Request ID</dt><dd>{d.request_id}</dd>
          <dt>Model</dt><dd>{d.model}</dd>
          <dt>API type</dt><dd>{d.api_type}</dd>
          <dt>Status</dt><dd style="color: {statusColor[d.status] ?? 'inherit'}">{d.status}</dd>
          <dt>Connection</dt><dd>{d.connection_id ?? 'none'}</dd>
          <dt>Started</dt><dd>{new Date(d.started_at_ms).toLocaleString()}</dd>
          <dt>Duration</dt><dd>{fmtDuration(d.duration_ms)}</dd>
        </dl>
      </div>
    {/if}
  </section>

  <section aria-labelledby="recent-heading">
    <h2 id="recent-heading">Recent requests ({data.requests.length})</h2>
    {#if data.requests.length === 0}
      <p>No requests recorded yet.</p>
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
              <td style="color: {statusColor[r.status] ?? 'inherit'}">{r.status}</td>
              <td>{fmtDuration(r.duration_ms)}</td>
              <td>{new Date(r.started_at_ms).toLocaleString()}</td>
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
  .mono { font-family: monospace; font-size: 0.8rem; }
  .error { color: #c0392b; font-size: 0.875rem; margin-top: 0.5rem; }
  .detail { margin-top: 0.75rem; background: #f9f9f9; border: 1px solid #ddd; border-radius: 4px; padding: 0.75rem 1rem; }
  dl { display: grid; grid-template-columns: max-content 1fr; gap: 0.25rem 1rem; font-size: 0.875rem; }
  dt { font-weight: 600; }
  dd { margin: 0; }
  a { color: #1a1a2e; font-size: 0.875rem; }
</style>
