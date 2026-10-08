<script lang="ts">
  import type { Snippet } from 'svelte';

  export interface StepShellProps {
    /** `narrow` = form-width column; `center` = centered hero/status layout. */
    variant?: 'default' | 'narrow' | 'center';
    title?: string;
    /** Rendered above the title (hero icon). */
    lead?: Snippet;
    sub?: Snippet;
    /** Footer row of buttons; `actionsEnd` right-aligns a single trailing action. */
    actions?: Snippet;
    actionsEnd?: boolean;
    children?: Snippet;
  }

  let { variant = 'default', title, lead, sub, actions, actionsEnd = false, children }: StepShellProps = $props();
</script>

<div class="step" class:narrow-step={variant === 'narrow'} class:center-step={variant === 'center'}>
  {#if lead}{@render lead()}{/if}
  {#if title}<h1 class="wizard-title">{title}</h1>{/if}
  {#if sub}<p class="wizard-sub">{@render sub()}</p>{/if}
  {#if children}{@render children()}{/if}
  {#if actions}
    <div class="action-row" class:justify-end={actionsEnd}>{@render actions()}</div>
  {/if}
</div>

<style>
  .step {
    width: 100%;
    max-width: calc(var(--space-8) * 10);
    animation: step-in var(--dur-3) var(--ease-out);
  }

  @keyframes step-in { from { opacity: 0; transform: translateY(var(--space-2)); } }

  .narrow-step { max-width: calc(var(--space-8) * 7.5); }

  .center-step {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: var(--space-4);
    padding-top: var(--space-4);
  }

  .wizard-title {
    font-size: var(--text-xl);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--tracking-tight);
    color: var(--text-1);
    margin: 0 0 var(--space-1);
    line-height: var(--leading-tight);
    text-wrap: balance;
  }

  .center-step .wizard-title { font-size: var(--text-hero); }

  .wizard-sub {
    font-size: var(--text-base);
    color: var(--text-2);
    margin: 0 0 var(--space-5);
    line-height: var(--leading);
    max-width: 52ch;
    overflow-wrap: anywhere;
  }

  .center-step .wizard-sub { margin-bottom: var(--space-3); }

  .wizard-sub :global(a:hover) { text-decoration: underline; }

  .action-row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    align-items: center;
    justify-content: space-between;
  }

  .justify-end { justify-content: flex-end; }

  @media (max-width: 480px) {
    .action-row > :global(*) { flex: 1 1 auto; }
  }
</style>
