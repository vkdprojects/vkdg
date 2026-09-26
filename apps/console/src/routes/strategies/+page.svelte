<main>
  <header>
    <h1>Routing Strategies</h1>
    <a href="/">Back to overview</a>
  </header>

  <section aria-labelledby="strategies-heading">
    <h2 id="strategies-heading">Available strategies</h2>
    <p class="intro">
      Each combo selects one strategy to decide how requests are distributed across its target connections.
    </p>

    <dl>
      <dt>round_robin</dt>
      <dd>
        Cycles through targets in order. Each target receives an equal share of traffic over time.
        Best for spreading load evenly across identical connections.
      </dd>

      <dt>fallback_chain</dt>
      <dd>
        Tries targets left to right. If the first target fails (error or circuit open), the next is tried.
        Continues until one succeeds or all are exhausted. Best for primary/backup setups.
      </dd>

      <dt>scored</dt>
      <dd>
        Ranks targets by a composite score derived from latency, error rate, and cost. The highest-scoring
        target wins each request. Score weights are controlled by the combo's <code>mode_pack</code> field
        — e.g. <code>low_latency</code>, <code>low_cost</code>, or <code>balanced</code>.
      </dd>

      <dt>fusion</dt>
      <dd>
        Sends the request to <em>all</em> targets simultaneously and returns the first successful response.
        Remaining in-flight requests are cancelled. Best when latency matters more than cost and targets
        are cheap.
      </dd>

      <dt>prompt_chain</dt>
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
          <td><code>balanced</code> <span class="default">default</span></td>
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
</main>

<style>
  main { max-width: 900px; margin: 2rem auto; font-family: system-ui, sans-serif; padding: 0 1rem; }
  header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 2rem; }
  h1 { margin: 0; font-size: 1.5rem; }
  h2 { margin-top: 0; font-size: 1.1rem; border-bottom: 1px solid #eee; padding-bottom: 0.25rem; }
  section { margin-bottom: 2.5rem; }
  p.intro { font-size: 0.9rem; color: #444; margin-top: 0; }
  dl { display: grid; grid-template-columns: max-content 1fr; gap: 0.75rem 1.5rem; align-items: baseline; }
  dt { font-weight: 700; font-family: monospace; font-size: 0.95rem; white-space: nowrap; }
  dd { margin: 0; font-size: 0.875rem; color: #333; }
  table { width: 100%; border-collapse: collapse; font-size: 0.875rem; }
  th, td { text-align: left; padding: 0.4rem 0.6rem; border-bottom: 1px solid #eee; }
  th { background: #f5f5f5; font-weight: 600; }
  code { font-size: 0.85em; background: #f0f0f0; padding: 0.1em 0.3em; border-radius: 3px; }
  a { color: #1a1a2e; font-size: 0.875rem; }
  p { font-size: 0.875rem; color: #444; }
  .default {
    display: inline-block;
    margin-left: 0.4rem;
    padding: 0.05rem 0.35rem;
    font-size: 0.7rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    background: #e8f4fd;
    color: #1a6fa8;
    border-radius: 3px;
    vertical-align: middle;
  }
</style>
