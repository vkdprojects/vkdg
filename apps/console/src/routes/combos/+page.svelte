<script lang="ts">
  import type { PageData } from './$types';
  export let data: PageData;
</script>

<main>
  <header>
    <h1>Combos</h1>
    <a href="/">Back to overview</a>
  </header>

  <section aria-labelledby="combos-heading">
    <h2 id="combos-heading">Combos ({data.combos.length})</h2>
    {#if data.combos.length === 0}
      <p class="muted">No combos configured.</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th scope="col">ID</th>
            <th scope="col">Strategy</th>
            <th scope="col">Patterns</th>
            <th scope="col">Targets</th>
            <th scope="col">Policies</th>
          </tr>
        </thead>
        <tbody>
          {#each data.combos as c (c.id)}
            <tr>
              <td>{c.id}</td>
              <td>{c.strategy}</td>
              <td>{c.match_patterns.join(', ') || '—'}</td>
              <td>{c.targets.join(', ') || '—'}</td>
              <td class="badges">
                {#if c.has_compression}<span class="badge compression">compression</span>{/if}
                {#if c.has_cache}<span class="badge cache">cache</span>{/if}
                {#if c.has_budget}<span class="badge budget">budget</span>{/if}
                {#if !c.has_compression && !c.has_cache && !c.has_budget}<span class="muted">—</span>{/if}
              </td>
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
  table { width: 100%; border-collapse: collapse; font-size: 0.9rem; }
  th, td { text-align: left; padding: 0.4rem 0.6rem; border-bottom: 1px solid #eee; }
  th { background: #f5f5f5; font-weight: 600; }
  .muted { color: #888; font-size: 0.875rem; }
  a { color: #1a1a2e; font-size: 0.875rem; }
  .badges { display: flex; gap: 0.35rem; flex-wrap: wrap; }
  .badge {
    display: inline-block;
    padding: 0.1rem 0.45rem;
    border-radius: 3px;
    font-size: 0.75rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }
  .badge.compression { background: #e8f4fd; color: #1a6fa8; }
  .badge.cache       { background: #e9fbe9; color: #1e7e34; }
  .badge.budget      { background: #fff3cd; color: #856404; }
</style>
