<script lang="ts">
  import { RefreshCwIcon } from 'lucide-svelte';
  import { m } from '$lib/paraglide/messages.js';
  import Button from './Button.svelte';

  interface Props {
    busy: boolean;
    onRefresh: () => void;
  }

  let { busy, onRefresh }: Props = $props();
</script>

<Button variant="outline" size="sm" onclick={onRefresh} disabled={busy} ariaLabel={m.common_refresh()}>
  <RefreshCwIcon size={14} aria-hidden="true" />
  {m.common_refresh()}
</Button>
