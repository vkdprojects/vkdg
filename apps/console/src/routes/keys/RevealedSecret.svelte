<script lang="ts">
  import { CopyButton } from '$lib/components/index.js';

  interface Props {
    secret: string;
    notice: string;
  }

  let { secret, notice }: Props = $props();
</script>

<div class="created-key" role="alert">
  <div class="created-header">
    <svg class="warning-icon" xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z"/>
      <path d="M12 9v4"/><path d="M12 17h.01"/>
    </svg>
    <strong class="created-notice">{notice}</strong>
  </div>
  <div class="key-box">
    <code class="key-value">{secret}</code>
  </div>
  <div class="key-copy-row">
    <CopyButton text={secret} />
  </div>
</div>

<style>
  /* Revealed secret: the one bold moment on this page */
  .created-key {
    margin-bottom: var(--space-5);
    padding: var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    border-radius: var(--radius-lg);
    border: var(--border-w) solid var(--accent);
    background: color-mix(in oklch, var(--accent) 14%, var(--bg-surface));
    box-shadow: var(--glow);
    animation: reveal var(--dur-3) var(--ease-spring);
  }

  @keyframes reveal {
    from { opacity: 0; transform: translateY(calc(-1 * var(--space-2))) scale(0.985); }
    to   { opacity: 1; transform: none; }
  }

  @media (prefers-reduced-motion: reduce) {
    .created-key { animation: none; }
  }

  .created-header {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
  }

  .warning-icon {
    color: var(--warning);
    flex-shrink: 0;
    margin-top: var(--space-0);
  }

  .created-notice {
    font-size: var(--text-base);
    color: var(--text-1);
    font-weight: var(--weight-semibold);
    line-height: var(--leading);
  }

  .key-box {
    background: var(--bg-inset);
    border: var(--border-w) solid var(--border-strong);
    border-radius: var(--radius);
    padding: var(--space-4);
    overflow-x: auto;
  }

  .key-value {
    display: block;
    background: none;
    border: 0;
    padding: 0;
    font-family: var(--font-mono);
    font-size: var(--text-md);
    color: var(--text-1);
    word-break: break-all;
    white-space: pre-wrap;
    user-select: all;
  }

  .key-copy-row {
    display: flex;
    justify-content: flex-end;
  }
</style>
