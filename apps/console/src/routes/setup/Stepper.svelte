<script lang="ts">
  import { Check } from 'lucide-svelte';
  import { m } from '$lib/paraglide/messages.js';

  export interface StepperProps {
    /** Current 1-based step. */
    step: number;
    total?: number;
  }

  let { step, total = 5 }: StepperProps = $props();
</script>

<div class="stepper" role="progressbar" aria-valuemin={1} aria-valuemax={total} aria-valuenow={step} aria-label={m.setup_progress_label()}>
  {#each { length: total } as _, i (i)}
    {@const s = i + 1}
    <div class="step-node" class:active={step === s} class:done={step > s}>
      <span class="step-circle">
        {#if step > s}<Check size={14} strokeWidth={3} aria-hidden="true" />{:else}{s}{/if}
      </span>
    </div>
  {/each}
</div>

<style>
  .stepper {
    display: flex;
    align-items: center;
    margin: 0 0 var(--space-6);
    width: 100%;
    max-width: calc(var(--space-8) * 5.5);
  }

  .step-node {
    display: flex;
    align-items: center;
    flex: 1;
  }

  .step-node:last-child { flex: 0; }

  /* connector line to the next node */
  .step-node:not(:last-child)::after {
    content: '';
    flex: 1;
    height: var(--focus-w);
    margin: 0 var(--space-2);
    border-radius: var(--radius-full);
    background: var(--border-strong);
    transition: background var(--dur-3) var(--ease-out);
  }

  .step-node.done:not(:last-child)::after {
    background: var(--accent);
  }

  .step-circle {
    display: grid;
    place-items: center;
    width: var(--control-h-sm);
    height: var(--control-h-sm);
    flex-shrink: 0;
    border-radius: var(--radius-full);
    border: 1px solid var(--border-strong);
    background: var(--bg-surface);
    color: var(--text-3);
    font-size: var(--text-xs);
    font-weight: var(--weight-semibold);
    font-variant-numeric: tabular-nums;
    transition: background var(--dur-2) var(--ease-out), color var(--dur-2) var(--ease-out),
                border-color var(--dur-2) var(--ease-out), box-shadow var(--dur-2) var(--ease-out),
                transform var(--dur-2) var(--ease-spring);
  }

  .step-node.active .step-circle {
    background: var(--accent-subtle);
    border-color: var(--accent);
    color: var(--accent);
    box-shadow: var(--ring);
    transform: scale(1.08);
  }

  .step-node.done .step-circle {
    background: var(--accent);
    border-color: transparent;
    color: var(--on-accent);
  }
</style>
