<script lang="ts">
  import { CopyButton } from '$lib/components/index.js';
  import { ArrowRight } from 'lucide-svelte';
  import { m } from '$lib/paraglide/messages.js';
  import StepShell from './StepShell.svelte';

  export interface StepReadyProps {
    endpoint: string;
  }

  let { endpoint }: StepReadyProps = $props();

  const clients = $derived([
    { label: 'Claude Code', command: `claude config set ANTHROPIC_BASE_URL ${endpoint}` },
    { label: 'Codex CLI', command: `codex --config openai-base-url=${endpoint}` },
  ]);
</script>

<StepShell variant="narrow" title={m.setup_ready_title()} actionsEnd>
  {#snippet sub()}{m.setup_ready_sub()}{/snippet}

  <div class="endpoint-row">
    <code class="endpoint-url mono">{endpoint}</code>
    <CopyButton text={endpoint} label={m.setup_copy_url()} />
  </div>

  <div class="divider">
    <span>{m.setup_configure_client()}</span>
  </div>

  <div class="code-blocks">
    {#each clients as c (c.label)}
      <div class="code-block">
        <span class="code-label">{c.label}</span>
        <div class="code-line">
          <code class="mono">{c.command}</code>
          <CopyButton text={c.command} label={m.common_copy()} />
        </div>
      </div>
    {/each}
  </div>

  {#snippet actions()}
    <a href="/" class="btn-link">
      {m.setup_go_to_dashboard()} <ArrowRight size={15} />
    </a>
  {/snippet}
</StepShell>

<style>
  .endpoint-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-2) var(--space-2) var(--space-4);
    background: var(--bg-inset);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    margin-bottom: var(--space-5);
  }

  .endpoint-url {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
    font-size: var(--text-sm);
    color: var(--accent);
    background: none;
    border: 0;
    padding: 0;
  }

  .divider {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-bottom: var(--space-4);
    color: var(--text-3);
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
  }

  .divider::before,
  .divider::after {
    content: '';
    flex: 1;
    height: var(--border-w);
    background: var(--border);
  }

  .code-blocks {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    margin-bottom: var(--space-6);
  }

  .code-block {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
  }

  .code-label {
    display: block;
    font-size: var(--text-xs);
    font-weight: var(--weight-semibold);
    color: var(--text-2);
    padding: var(--space-3) var(--space-4) 0;
  }

  .code-line {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3) var(--space-3) var(--space-4);
  }

  .code-line code {
    flex: 1 1 calc(var(--space-8) * 3.5);
    min-width: 0;
    font-size: var(--text-sm);
    color: var(--text-1);
    overflow-wrap: anywhere;
  }

  .btn-link {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    min-height: var(--control-h-lg);
    padding: var(--space-2) var(--space-5);
    background: var(--accent);
    color: var(--on-accent);
    border-radius: var(--radius);
    font-size: var(--text-md);
    font-weight: var(--weight-medium);
    white-space: nowrap;
    box-shadow: var(--shadow-1);
  }

  .btn-link:hover {
    background: var(--accent-hover);
    color: var(--on-accent);
    transform: var(--lift);
  }

  .btn-link:active { transform: var(--press); }

  @media (max-width: 480px) {
    .btn-link { width: 100%; }
  }

  @media (prefers-reduced-motion: reduce) {
    .btn-link:hover { transform: none; }
  }
</style>
