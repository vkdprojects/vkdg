<script lang="ts">
  interface Props {
    size?: 'sm' | 'md' | 'lg';
  }

  let { size = 'md' }: Props = $props();
</script>

<span class="spinner {size}" role="status" aria-label="Loading"></span>

<style>
  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .spinner {
    display: inline-block;
    border-radius: var(--radius-full);
    border: var(--focus-w) solid var(--accent-subtle);
    border-top-color: var(--accent);
    border-right-color: var(--accent-strong);
    animation: spin var(--dur-spin) linear infinite;
    flex-shrink: 0;
  }

  .sm { width: var(--icon-sm); height: var(--icon-sm); }
  .md { width: var(--icon-lg); height: var(--icon-lg); }
  .lg { width: var(--icon-xl); height: var(--icon-xl); border-width: calc(var(--focus-w) * 1.5); }

  @media (prefers-reduced-motion: reduce) {
    /* Global rule freezes animation; keep a visible, non-rotating cue. */
    .spinner { border-color: var(--accent); opacity: 0.7; }
  }
</style>
