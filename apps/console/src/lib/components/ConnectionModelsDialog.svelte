<script lang="ts">
  import { Dialog } from 'bits-ui';
  import { XIcon } from 'lucide-svelte';
  import { m } from '$lib/paraglide/messages.js';

  interface Props {
    open: boolean;
    connectionId: string;
    models: string[];
  }

  let { open = $bindable(false), connectionId, models }: Props = $props();

  const isPattern = (id: string) => /[*?]/.test(id);
</script>

<Dialog.Root bind:open>
  <Dialog.Portal>
    <Dialog.Overlay class="dialog-overlay" />
    <Dialog.Content class="dialog-content" aria-describedby={undefined}>
      <div class="dialog-header">
        <Dialog.Title class="dialog-title">
          {m.connection_models_dialog_title({ id: connectionId, count: models.length })}
        </Dialog.Title>
        <button class="dialog-close" aria-label={m.common_close()} onclick={() => (open = false)}>
          <XIcon size={16} aria-hidden="true" />
        </button>
      </div>
      <ul class="dialog-body models-list">
        {#each models as id (id)}
          <li class="models-item" class:pattern={isPattern(id)}>{id}</li>
        {/each}
      </ul>
    </Dialog.Content>
  </Dialog.Portal>
</Dialog.Root>

<style>
  /* Portaled content sits outside this component's DOM scope, hence :global. */
  :global(.models-list) {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
    list-style: none;
  }
  :global(.models-item) {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius);
    background: var(--bg-inset);
    color: var(--text-1);
    overflow-wrap: anywhere;
  }
  :global(.models-item.pattern) { color: var(--text-2); }
</style>
