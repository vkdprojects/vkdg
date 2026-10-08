<script lang="ts">
  import { Badge } from '$lib/components/index.js';
  import { ArrowRight } from 'lucide-svelte';
  import { m } from '$lib/paraglide/messages.js';
  import StepShell from './StepShell.svelte';
  import type { SetupProvider } from './providers.js';

  export interface StepProviderProps {
    providers: SetupProvider[];
    onpick: (p: SetupProvider) => void;
  }

  let { providers, onpick }: StepProviderProps = $props();
</script>

<StepShell title={m.setup_pick_provider_title()}>
  <div class="provider-grid">
    {#each providers as p (p.id)}
      <button class="provider-card" onclick={() => onpick(p)}>
        <span class="provider-dot" aria-hidden="true"></span>
        <div class="provider-info">
          <span class="provider-name">
            {p.label}
            {#if p.free}
              <Badge status="success" label={m.setup_free_tier_badge()} />
            {/if}
          </span>
          <span class="provider-desc">{p.desc}</span>
        </div>
        <ArrowRight size={13} class="provider-arrow" />
      </button>
    {/each}
  </div>
</StepShell>

<style>
  .provider-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, calc(var(--space-8) * 3.75)), 1fr));
    gap: var(--space-3);
    margin-top: var(--space-5);
  }

  .provider-card {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-height: var(--control-h-lg);
    padding: var(--space-3) var(--space-4);
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    box-shadow: var(--shadow-1);
    color: inherit;
    font: inherit;
    cursor: pointer;
    text-align: left;
  }

  .provider-card:hover {
    border-color: var(--accent-strong);
    background: var(--bg-elevated);
    transform: var(--lift);
  }

  .provider-card:active { transform: var(--press); }

  .provider-dot {
    width: var(--dot-size);
    height: var(--dot-size);
    border-radius: var(--radius-full);
    background: var(--accent);
    flex-shrink: 0;
  }

  .provider-info {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: var(--space-0);
    min-width: 0;
  }

  .provider-name {
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    color: var(--text-1);
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .provider-desc {
    font-size: var(--text-xs);
    color: var(--text-3);
  }

  :global(.provider-arrow) {
    color: var(--text-3);
    flex-shrink: 0;
    transition: transform var(--dur-2) var(--ease-out), color var(--dur-1) ease;
  }

  .provider-card:hover :global(.provider-arrow) {
    color: var(--accent);
    transform: translateX(var(--space-1));
  }

  @media (prefers-reduced-motion: reduce) {
    .provider-card:hover { transform: none; }
  }
</style>
