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

<button type="button" class="copy-btn" class:copied onclick={copy} aria-label={copied ? 'Copied' : label}>
  <span class="glyph" aria-hidden="true">
    <svg class="ico ico-copy" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <rect x="9" y="9" width="13" height="13" rx="2" ry="2"/>
      <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>
    </svg>
    <svg class="ico ico-check" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
      <polyline points="20 6 9 17 4 12"/>
    </svg>
  </span>
  <span class="text" aria-live="polite">{copied ? 'Copied!' : label}</span>
</button>

<style>
  .copy-btn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    min-height: var(--control-h-sm);
    padding: 0 var(--space-3);
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text-2);
    cursor: pointer;
    font: inherit;
    font-size: var(--text-xs);
    font-weight: var(--weight-medium);
    white-space: nowrap;
    flex-shrink: 0;
  }

  .copy-btn:hover {
    background: var(--bg-hover);
    border-color: var(--border-strong);
    color: var(--text-1);
  }

  .copy-btn:active {
    transform: var(--press);
  }

  .copy-btn.copied,
  .copy-btn.copied:hover {
    background: var(--success-subtle);
    border-color: color-mix(in oklch, var(--success) 40%, transparent);
    color: var(--success);
  }

  .glyph {
    position: relative;
    display: grid;
    place-items: center;
    width: var(--icon-sm);
    height: var(--icon-sm);
  }

  .ico {
    grid-area: 1 / 1;
    transition: opacity var(--dur-2) var(--ease-out), transform var(--dur-2) var(--ease-spring);
  }

  .ico-check {
    opacity: 0;
    transform: scale(0.4) rotate(-30deg);
  }

  .copied .ico-copy {
    opacity: 0;
    transform: scale(0.4) rotate(30deg);
  }

  .copied .ico-check {
    opacity: 1;
    transform: none;
  }
</style>
