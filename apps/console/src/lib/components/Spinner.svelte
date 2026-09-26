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
    border-radius: 50%;
    border: 2px solid var(--border-strong);
    border-top-color: var(--accent);
    animation: spin 0.7s linear infinite;
    flex-shrink: 0;
  }

  .sm { width: 14px; height: 14px; }
  .md { width: 20px; height: 20px; }
  .lg { width: 28px; height: 28px; }
</style>
