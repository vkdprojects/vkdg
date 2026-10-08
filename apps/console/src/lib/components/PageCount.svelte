<script lang="ts">
  interface Props {
    /** Number of items shown on the page, rendered next to the `<h1>`. */
    value: number;
  }

  let { value }: Props = $props();
</script>

<span class="count mono">{value}</span>

<style>
  .count {
    color: var(--text-3);
    font-weight: var(--weight-regular);
    font-size: var(--text-md);
    margin-left: var(--space-1);
  }
</style>
