<script lang="ts">
  import { Button } from '$lib/components/index.js';
  import { Zap, ArrowRight } from 'lucide-svelte';
  import { m } from '$lib/paraglide/messages.js';
  import StepShell from './StepShell.svelte';

  export interface StepWelcomeProps {
    onstart: () => void;
  }

  let { onstart }: StepWelcomeProps = $props();
</script>

<StepShell variant="center" title={m.setup_welcome_title()}>
  {#snippet lead()}
    <div class="hero-icon">
      <Zap size={32} strokeWidth={1.5} />
    </div>
  {/snippet}
  {#snippet sub()}{m.setup_welcome_sub()}{/snippet}
  <Button size="lg" onclick={onstart}>
    {m.setup_get_started()} <ArrowRight size={15} />
  </Button>
</StepShell>

<style>
  .hero-icon {
    width: calc(var(--space-8) * 1.125);
    height: calc(var(--space-8) * 1.125);
    border-radius: var(--radius-lg);
    background: var(--accent);
    display: grid;
    place-items: center;
    color: var(--on-accent);
    box-shadow: var(--glow);
    margin-bottom: var(--space-2);
  }
</style>
