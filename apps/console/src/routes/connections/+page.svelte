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
    <h1>Connections</h1>
    <a href="/">Back to overview</a>
  </header>

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
</main>

<style>
  main { max-width: 900px; margin: 2rem auto; font-family: system-ui, sans-serif; padding: 0 1rem; }
  header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 2rem; }
  h1 { margin: 0; font-size: 1.5rem; }
  table { width: 100%; border-collapse: collapse; font-size: 0.9rem; }
  th, td { text-align: left; padding: 0.4rem 0.6rem; border-bottom: 1px solid #eee; }
  th { background: #f5f5f5; font-weight: 600; }
  a { color: #1a1a2e; font-size: 0.875rem; }
</style>
