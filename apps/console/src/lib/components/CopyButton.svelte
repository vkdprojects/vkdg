<script lang="ts">
  interface Props {
    text: string;
    label?: string;
  }

  let { text, label = 'Copy' }: Props = $props();

  let copied = $state(false);
  let timer: ReturnType<typeof setTimeout>;

  function copy() {
    navigator.clipboard.writeText(text).then(() => {
      copied = true;
      clearTimeout(timer);
      timer = setTimeout(() => (copied = false), 2000);
    });
  }
</script>

<button type="button" class="copy-btn" onclick={copy} aria-label={copied ? 'Copied' : label}>
  {#if copied}
    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
      <polyline points="20 6 9 17 4 12"/>
    </svg>
  {:else}
    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <rect x="9" y="9" width="13" height="13" rx="2" ry="2"/>
      <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>
    </svg>
  {/if}
  {copied ? 'Copied!' : label}
</button>

<style>
  .copy-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 0.3rem 0.625rem;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--text-2);
    cursor: pointer;
    font-size: 0.75rem;
    font-weight: 500;
    transition: background 0.1s, color 0.1s;
  }

  .copy-btn:hover {
    background: var(--bg-hover);
    color: var(--text-1);
  }

  .copy-btn:has(polyline) {
    color: var(--success);
  }
</style>
