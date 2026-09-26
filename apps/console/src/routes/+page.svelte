<script lang="ts">
  import type { PageData } from './$types';
  export let data: PageData;

  const statusColor: Record<string, string> = {
    healthy: '#27ae60',
    degraded: '#e67e22',
    circuit_open: '#c0392b',
    cooldown: '#8e44ad',
  };
</script>

<main>
  <header>
    <h1>VKDG Console</h1>
    <form method="POST" action="/logout">
      <button type="submit">Sign out</button>
    </form>
  </header>

  <section aria-labelledby="system-heading">
    <h2 id="system-heading">System</h2>
    <dl>
      <dt>Version</dt>
      <dd>{data.system.version}</dd>
      <dt>Status</dt>
      <dd>{data.system.status}</dd>
      <dt>Config revision</dt>
      <dd>{data.system.config_revision}</dd>
      <dt>Uptime</dt>
      <dd>{data.system.uptime_secs}s</dd>
      <dt>Active requests</dt>
      <dd>{data.system.active_requests}</dd>
    </dl>
  </section>

  <section aria-labelledby="connections-heading">
    <h2 id="connections-heading">Connections ({data.connections.length})</h2>
    {#if data.connections.length === 0}
      <p>No connections configured.</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th scope="col">ID</th>
            <th scope="col">Provider</th>
            <th scope="col">Status</th>
            <th scope="col">Models</th>
            <th scope="col">Active requests</th>
          </tr>
        </thead>
        <tbody>
          {#each data.connections as conn (conn.id)}
            <tr>
              <td>{conn.id}</td>
              <td>{conn.provider}</td>
              <td style="color: {statusColor[conn.status] ?? 'inherit'}">{conn.status}</td>
              <td>{conn.model_count}</td>
              <td>{conn.active_requests}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>
</main>

<style>
  main {
    max-width: 900px;
    margin: 2rem auto;
    font-family: system-ui, sans-serif;
    padding: 0 1rem;
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 2rem;
  }
  h1 { margin: 0; font-size: 1.5rem; }
  h2 { margin-top: 0; font-size: 1.1rem; border-bottom: 1px solid #eee; padding-bottom: 0.25rem; }
  dl { display: grid; grid-template-columns: max-content 1fr; gap: 0.25rem 1rem; }
  dt { font-weight: 600; }
  dd { margin: 0; }
  section { margin-bottom: 2rem; }
  table { width: 100%; border-collapse: collapse; font-size: 0.9rem; }
  th, td { text-align: left; padding: 0.4rem 0.6rem; border-bottom: 1px solid #eee; }
  th { background: #f5f5f5; font-weight: 600; }
  button {
    padding: 0.4rem 0.75rem;
    background: #1a1a2e;
    color: #fff;
    border: none;
    border-radius: 4px;
    cursor: pointer;
    font-size: 0.875rem;
  }
</style>
