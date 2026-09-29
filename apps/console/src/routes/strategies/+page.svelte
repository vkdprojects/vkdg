<script lang="ts">
  import { Card, Badge } from '$lib/components/index.js';
  import { m } from '$lib/paraglide/messages.js';
  // No data load needed — static content
</script>

<div class="page">
  <div class="page-header">
    <h1 class="page-title">{m.nav_strategies()}</h1>
  </div>

  <section aria-labelledby="strategies-heading">
    <div class="section-head">
      <h2 id="strategies-heading" class="section-title">{m.strategies_available()}</h2>
    </div>
    <p class="intro">{m.strategies_available_desc()}</p>

    <div class="strategy-grid">
      <Card>
        <div class="strategy-name mono">round_robin</div>
        <p class="strategy-desc">
          Cycles through targets in order. Each target receives an equal share of traffic over time.
          Best for spreading load evenly across identical connections.
        </p>
      </Card>

      <Card>
        <div class="strategy-name mono">fallback_chain</div>
        <p class="strategy-desc">
          Tries targets left to right. If the first target fails (error or circuit open), the next is tried.
          Continues until one succeeds or all are exhausted. Best for primary/backup setups.
        </p>
      </Card>

      <Card>
        <div class="strategy-name mono">scored</div>
        <p class="strategy-desc">
          Ranks targets by a composite score derived from latency, error rate, and cost. The highest-scoring
          target wins each request. Score weights are controlled by the combo's <code>mode_pack</code> field
          — e.g. <code>low_latency</code>, <code>low_cost</code>, or <code>balanced</code>.
        </p>
      </Card>

      <Card>
        <div class="strategy-name mono">fusion</div>
        <p class="strategy-desc">
          Sends the request to <em>all</em> targets simultaneously and returns the first successful response.
          Remaining in-flight requests are cancelled. Best when latency matters more than cost and targets
          are cheap.
        </p>
      </Card>

      <Card>
        <div class="strategy-name mono">prompt_chain</div>
        <p class="strategy-desc">
          Routes each turn of a multi-turn conversation through a defined sequence of targets. Step N's
          output is injected as context for step N+1. Useful for multi-model pipelines such as
          draft → review → refine.
        </p>
      </Card>
    </div>
  </section>

  <section aria-labelledby="mode-packs-heading">
    <div class="section-head">
      <h2 id="mode-packs-heading" class="section-title">{m.strategies_mode_packs()}</h2>
    </div>
    <p class="intro">{m.strategies_mode_packs_desc()}</p>
    <Card padding="0">
      <table>
        <thead>
          <tr>
            <th scope="col">{m.strategies_pack()}</th>
            <th scope="col">{m.strategies_latency_weight()}</th>
            <th scope="col">{m.strategies_error_rate_weight()}</th>
            <th scope="col">{m.strategies_cost_weight()}</th>
            <th scope="col">{m.strategies_use_case()}</th>
          </tr>
        </thead>
        <tbody>
          <tr>
            <td>
              <span class="mono">balanced</span>
              <Badge status="default" label={m.strategies_default()} />
            </td>
            <td class="mono">1×</td><td class="mono">1×</td><td class="mono">1×</td>
            <td>General-purpose traffic</td>
          </tr>
          <tr>
            <td class="mono">low_latency</td>
            <td class="mono">3×</td><td class="mono">1×</td><td class="mono">0.25×</td>
            <td>Interactive / streaming chat</td>
          </tr>
          <tr>
            <td class="mono">low_cost</td>
            <td class="mono">0.25×</td><td class="mono">1×</td><td class="mono">3×</td>
            <td>Batch jobs, embeddings</td>
          </tr>
          <tr>
            <td class="mono">reliability</td>
            <td class="mono">0.5×</td><td class="mono">4×</td><td class="mono">0.5×</td>
            <td>Critical pipelines, low tolerance for errors</td>
          </tr>
        </tbody>
      </table>
    </Card>
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

  .section-head {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    border-bottom: 1px solid var(--border);
    padding-bottom: 0.5rem;
    margin-bottom: 0.75rem;
  }

  .section-title {
    margin: 0;
    border: none;
    padding: 0;
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-1);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .intro {
    font-size: var(--text-sm);
    color: var(--text-3);
    margin-top: 0;
  }

  .strategy-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: 12px;
    margin-bottom: 0.5rem;
  }

  .strategy-name {
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--accent);
    margin-bottom: 0.5rem;
  }

  .strategy-desc {
    font-size: var(--text-sm);
    color: var(--text-2);
    margin: 0;
  }

  table {
    margin: 0;
  }

  th:first-child,
  td:first-child {
    padding-left: 1.25rem;
  }

  th:last-child,
  td:last-child {
    padding-right: 1.25rem;
  }
</style>
