<script lang="ts">
  import { Badge } from '$lib/components/index.js';
  import { m } from '$lib/paraglide/messages.js';
  // No data load needed — static content. Selection is local, for browsing the reference.

  const strategies = [
    {
      id: 'round_robin',
      desc: 'Cycles through targets in order. Each target receives an equal share of traffic over time. Best for spreading load evenly across identical connections.',
    },
    {
      id: 'fallback_chain',
      desc: 'Tries targets left to right. If the first target fails (error or circuit open), the next is tried. Continues until one succeeds or all are exhausted. Best for primary/backup setups.',
    },
    {
      id: 'scored',
      desc: "Ranks targets by a composite score derived from latency, error rate, and cost. The highest-scoring target wins each request. Score weights are controlled by the combo's mode_pack field — e.g. low_latency, low_cost, or balanced.",
    },
    {
      id: 'fusion',
      desc: 'Sends the request to all targets simultaneously and returns the first successful response. Remaining in-flight requests are cancelled. Best when latency matters more than cost and targets are cheap.',
    },
    {
      id: 'prompt_chain',
      desc: "Routes each turn of a multi-turn conversation through a defined sequence of targets. Step N's output is injected as context for step N+1. Useful for multi-model pipelines such as draft → review → refine.",
    },
  ];

  let selected = $state('round_robin');
</script>

<div class="page">
  <div class="page-header">
    <div>
      <h1>{m.nav_strategies()}</h1>
      <p>{m.strategies_available_desc()}</p>
    </div>
  </div>

  <section aria-labelledby="strategies-heading">
    <h2 id="strategies-heading" class="section-title">{m.strategies_available()}</h2>

    <div class="strategy-grid" role="radiogroup" aria-labelledby="strategies-heading">
      {#each strategies as s (s.id)}
        <label class="strategy-card" class:active={selected === s.id}>
          <input type="radio" name="strategy" value={s.id} bind:group={selected} class="sr-only" />
          <span class="strategy-top">
            <span class="strategy-name mono">{s.id}</span>
            <span class="check" aria-hidden="true">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg>
            </span>
          </span>
          <span class="strategy-desc">{s.desc}</span>
        </label>
      {/each}
    </div>
  </section>

  <section class="panel" aria-labelledby="mode-packs-heading">
    <div class="panel-head">
      <div>
        <h2 id="mode-packs-heading">{m.strategies_mode_packs()}</h2>
        <p class="head-desc">{m.strategies_mode_packs_desc()}</p>
      </div>
    </div>
    <div class="table-wrap">
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
            <td class="pack">
              <span class="mono">balanced</span>
              <Badge status="default" label={m.strategies_default()} />
            </td>
            <td class="mono">1×</td><td class="mono">1×</td><td class="mono">1×</td>
            <td>General-purpose traffic</td>
          </tr>
          <tr>
            <td class="pack"><span class="mono">low_latency</span></td>
            <td class="mono">3×</td><td class="mono">1×</td><td class="mono">0.25×</td>
            <td>Interactive / streaming chat</td>
          </tr>
          <tr>
            <td class="pack"><span class="mono">low_cost</span></td>
            <td class="mono">0.25×</td><td class="mono">1×</td><td class="mono">3×</td>
            <td>Batch jobs, embeddings</td>
          </tr>
          <tr>
            <td class="pack"><span class="mono">reliability</span></td>
            <td class="mono">0.5×</td><td class="mono">4×</td><td class="mono">0.5×</td>
            <td>Critical pipelines, low tolerance for errors</td>
          </tr>
        </tbody>
      </table>
    </div>
  </section>
</div>

<style>
  .section-title {
    margin-bottom: var(--space-4);
  }

  .head-desc {
    margin: var(--space-1) 0 0;
    font-size: var(--text-sm);
    color: var(--text-3);
  }

  .strategy-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, var(--col-lg)), 1fr));
    gap: var(--space-4);
  }

  .strategy-card {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-5);
    background: var(--bg-surface);
    border: var(--border-w) solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-1);
    cursor: pointer;
    font-weight: var(--weight-regular);
    transition:
      border-color var(--dur-2) var(--ease-out),
      box-shadow var(--dur-2) var(--ease-out),
      background-color var(--dur-2) var(--ease-out),
      transform var(--dur-2) var(--ease-out);
  }

  .strategy-card:hover {
    border-color: var(--border-strong);
    transform: var(--lift);
    box-shadow: var(--shadow-2);
  }

  .strategy-card:active {
    transform: var(--press);
  }

  .strategy-card.active {
    border-color: var(--accent);
    background: color-mix(in oklch, var(--accent) 6%, var(--bg-surface));
    box-shadow: 0 0 0 var(--focus-w) var(--accent), var(--glow);
  }

  .strategy-card:has(input:focus-visible) {
    outline: var(--focus-w) solid var(--accent);
    outline-offset: 3px;
  }

  .strategy-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .strategy-name {
    font-size: var(--text-base);
    font-weight: var(--weight-semibold);
    color: var(--text-1);
    overflow-wrap: anywhere;
  }

  .strategy-card.active .strategy-name {
    color: var(--accent);
  }

  .check {
    flex: none;
    display: inline-grid;
    place-items: center;
    width: var(--icon-lg);
    height: var(--icon-lg);
    border-radius: var(--radius-full);
    border: var(--focus-w) solid var(--border-strong);
    color: transparent;
    transition:
      background-color var(--dur-2) var(--ease-out),
      border-color var(--dur-2) var(--ease-out),
      color var(--dur-2) var(--ease-out),
      transform var(--dur-2) var(--ease-spring);
  }

  .strategy-card.active .check {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent);
    transform: scale(1.05);
  }

  .strategy-desc {
    font-size: var(--text-sm);
    line-height: var(--leading);
    color: var(--text-2);
  }

  .pack {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
    color: var(--text-1);
  }

  @media (prefers-reduced-motion: reduce) {
    .strategy-card:hover { transform: none; }
  }
</style>
