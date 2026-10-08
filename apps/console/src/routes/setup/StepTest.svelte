<script lang="ts">
  import { Button, Spinner } from '$lib/components/index.js';
  import { ArrowRight, CheckCircle2 } from 'lucide-svelte';
  import { m } from '$lib/paraglide/messages.js';
  import StepShell from './StepShell.svelte';

  export interface StepTestProps {
    testing: boolean;
    oncontinue: () => void;
  }

  let { testing, oncontinue }: StepTestProps = $props();
</script>

<StepShell variant="center">
  {#if testing}
    <Spinner size="lg" />
    <p class="testing-label">{m.setup_testing_connection()}</p>
  {:else}
    <div class="success-icon">
      <CheckCircle2 size={40} strokeWidth={1.5} />
    </div>
    <p class="testing-label success-label">{m.setup_connected_models({ n: 3 })}</p>
    <Button size="lg" onclick={oncontinue}>
      {m.common_continue()} <ArrowRight size={15} />
    </Button>
  {/if}
</StepShell>

<style>
  .testing-label {
    font-size: var(--text-md);
    color: var(--text-2);
    margin: 0;
  }

  .success-icon {
    display: grid;
    place-items: center;
    width: calc(var(--space-8) * 1.125);
    height: calc(var(--space-8) * 1.125);
    border-radius: var(--radius-full);
    background: var(--success-subtle);
    color: var(--success);
    animation: pop-in var(--dur-3) var(--ease-spring);
  }

  @keyframes pop-in { from { opacity: 0; transform: scale(0.5); } }

  .success-label {
    color: var(--success);
    font-weight: var(--weight-medium);
  }
</style>
