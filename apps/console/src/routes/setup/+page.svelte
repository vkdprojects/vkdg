<script lang="ts">
  import Stepper from './Stepper.svelte';
  import StepWelcome from './StepWelcome.svelte';
  import StepProvider from './StepProvider.svelte';
  import StepApiKey from './StepApiKey.svelte';
  import StepTest from './StepTest.svelte';
  import StepReady from './StepReady.svelte';
  import { setupProviders } from './providers.js';
  import type { SetupProvider } from './providers.js';

  const ENDPOINT = 'http://localhost:8080';
  const providers = setupProviders();

  let step = $state(1);
  let provider = $state<SetupProvider | null>(null);
  let apiKey = $state('');
  let testing = $state(false);

  function pickProvider(p: SetupProvider) {
    provider = p;
    step = 3;
  }

  async function testConnection() {
    testing = true;
    step = 4;
    await new Promise((r) => setTimeout(r, 1500));
    testing = false;
  }
</script>

<div class="page wizard-wrap">
  <Stepper {step} />

  {#if step === 1}
    <StepWelcome onstart={() => (step = 2)} />
  {:else if step === 2}
    <StepProvider {providers} onpick={pickProvider} />
  {:else if step === 3}
    <StepApiKey
      providerLabel={provider?.label ?? ''}
      providerUrl={provider?.url ?? ''}
      bind:apiKey
      onback={() => (step = 2)}
      ontest={testConnection}
    />
  {:else if step === 4}
    <StepTest {testing} oncontinue={() => (step = 5)} />
  {:else if step === 5}
    <StepReady endpoint={ENDPOINT} />
  {/if}
</div>

<style>
  .wizard-wrap {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding-top: var(--space-6);
  }
</style>
