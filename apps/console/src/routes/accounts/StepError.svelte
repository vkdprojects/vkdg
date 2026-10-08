<script lang="ts">
  import { AlertTriangle } from 'lucide-svelte';
  import { m } from '$lib/paraglide/messages.js';
  import { Button } from '$lib/components/index.js';

  interface Props {
    message: string;
    onclose: () => void;
    onretry: () => void;
  }

  let { message, onclose, onretry }: Props = $props();
</script>

<div class="error" role="alert">
  <AlertTriangle size={16} aria-hidden="true" />
  <p>{message}</p>
</div>
<div class="dialog-footer">
  <Button variant="outline" onclick={onclose}>{m.acct_close()}</Button>
  <Button onclick={onretry}>{m.acct_retry()}</Button>
</div>

<style>
  .error {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    color: var(--danger);
    background: var(--danger-subtle);
    border: var(--border-w) solid color-mix(in oklch, var(--danger) 34%, transparent);
    border-radius: var(--radius);
    padding: var(--space-3) var(--space-4);
  }

  .error :global(svg) {
    flex-shrink: 0;
    margin-top: var(--space-0);
  }

  .error p {
    font-size: var(--text-base);
    margin: 0;
    color: var(--text-1);
    overflow-wrap: anywhere;
  }
</style>
