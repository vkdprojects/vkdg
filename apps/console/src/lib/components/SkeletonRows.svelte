<script lang="ts">
  interface Props {
    /** Placeholder rows to draw. */
    rows?: number;
    /** Set when this block is the busy region itself (no busy ancestor). */
    busy?: boolean;
  }

  let { rows = 3, busy = false }: Props = $props();
</script>

<div class="panel-body skeleton-rows" aria-busy={busy || undefined}>
  {#each { length: rows } as _, i (i)}
    <div class="skeleton row"></div>
  {/each}
</div>

<style>
  .skeleton-rows {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .skeleton.row {
    height: var(--control-h);
  }
</style>
