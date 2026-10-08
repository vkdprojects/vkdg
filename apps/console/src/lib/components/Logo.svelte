<script lang="ts">
  interface Props {
    /** Width in pixels. Height scales automatically (square-ish character). */
    size?: number;
    class?: string;
    /** Soft accent glow behind the mark; use for hero placements only. */
    glow?: boolean;
  }

  let { size = 32, class: className = '', glow = false }: Props = $props();
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
  class="logo {className}"
  class:glow
  aria-hidden="true"
/>

<style>
  .logo {
    display: block;
    flex-shrink: 0;
    object-fit: contain;
  }

  .glow {
    filter: drop-shadow(0 var(--space-1) var(--space-4) color-mix(in oklch, var(--accent) 55%, transparent));
  }
</style>
