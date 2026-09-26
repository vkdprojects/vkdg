<script lang="ts">
  interface Props {
    /** Width in pixels. Height scales automatically (square-ish character). */
    size?: number;
    class?: string;
  }

  let { size = 32, class: className = '' }: Props = $props();
</script>

<!--
  Single source of truth: static/logo.svg
  Editing the logo = edit static/logo.svg only. No other files need changing.
-->
<img
  src="/logo.svg"
  alt="VKDG"
  width={size}
  height={size}
  class={className}
  style="display:block;flex-shrink:0;object-fit:contain"
  aria-hidden="true"
/>
