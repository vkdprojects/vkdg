<script lang="ts">
  // No data load needed — static content
</script>

<div class="page">
  <h1>Routing Strategies</h1>

  <section aria-labelledby="strategies-heading">
    <h2 id="strategies-heading">Available strategies</h2>
    <p class="intro">
      Each combo selects one strategy to decide how requests are distributed across its target connections.
    </p>

    <dl class="strategy-list">
      <dt><code>round_robin</code></dt>
      <dd>
        Cycles through targets in order. Each target receives an equal share of traffic over time.
        Best for spreading load evenly across identical connections.
      </dd>

      <dt><code>fallback_chain</code></dt>
      <dd>
        Tries targets left to right. If the first target fails (error or circuit open), the next is tried.
        Continues until one succeeds or all are exhausted. Best for primary/backup setups.
      </dd>

      <dt><code>scored</code></dt>
      <dd>
        Ranks targets by a composite score derived from latency, error rate, and cost. The highest-scoring
        target wins each request. Score weights are controlled by the combo's <code>mode_pack</code> field
        — e.g. <code>low_latency</code>, <code>low_cost</code>, or <code>balanced</code>.
      </dd>

      <dt><code>fusion</code></dt>
      <dd>
        Sends the request to <em>all</em> targets simultaneously and returns the first successful response.
        Remaining in-flight requests are cancelled. Best when latency matters more than cost and targets
        are cheap.
      </dd>

      <dt><code>prompt_chain</code></dt>
      <dd>
        Routes each turn of a multi-turn conversation through a defined sequence of targets. Step N's
        output is injected as context for step N+1. Useful for multi-model pipelines such as
        draft → review → refine.
      </dd>
    </dl>
  </section>

  <section aria-labelledby="mode-packs-heading">
    <h2 id="mode-packs-heading">Mode packs (scored strategy)</h2>
    <p>
      Mode packs tune the weight given to each scoring dimension. Set <code>mode_pack</code> on a combo
      to override the default balanced weights.
    </p>
    <table>
      <thead>
        <tr>
          <th scope="col">Pack</th>
          <th scope="col">Latency weight</th>
          <th scope="col">Error-rate weight</th>
          <th scope="col">Cost weight</th>
          <th scope="col">Use case</th>
        </tr>
      </thead>
      <tbody>
        <tr>
          <td><code>balanced</code> <span class="default-badge">default</span></td>
          <td>1×</td><td>1×</td><td>1×</td>
          <td>General-purpose traffic</td>
        </tr>
        <tr>
          <td><code>low_latency</code></td>
          <td>3×</td><td>1×</td><td>0.25×</td>
          <td>Interactive / streaming chat</td>
        </tr>
        <tr>
          <td><code>low_cost</code></td>
          <td>0.25×</td><td>1×</td><td>3×</td>
          <td>Batch jobs, embeddings</td>
        </tr>
        <tr>
          <td><code>reliability</code></td>
          <td>0.5×</td><td>4×</td><td>0.5×</td>
          <td>Critical pipelines, low tolerance for errors</td>
        </tr>
      </tbody>
    </table>
  </section>
</div>

<style>
  .intro {
    font-size: 0.875rem;
    color: var(--text-3);
    margin-top: 0;
  }

  .strategy-list {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 0.875rem 1.5rem;
    align-items: baseline;
    margin-bottom: 0.5rem;
  }

  .strategy-list dt {
    font-weight: 700;
    white-space: nowrap;
  }

  .strategy-list dd {
    margin: 0;
    font-size: 0.875rem;
    color: var(--text-2);
  }

  .default-badge {
    display: inline-block;
    margin-left: 0.35rem;
    padding: 0.05rem 0.35rem;
    font-size: 0.7rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    background: color-mix(in oklch, var(--accent) 15%, transparent);
    color: var(--accent);
    border-radius: 3px;
    vertical-align: middle;
  }
</style>
