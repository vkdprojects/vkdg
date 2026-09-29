<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    variant?: 'primary' | 'ghost' | 'danger' | 'outline';
    size?: 'sm' | 'md' | 'lg';
    type?: 'button' | 'submit' | 'reset';
    disabled?: boolean;
    onclick?: () => void;
    /** Accessible name when the visible text alone is ambiguous (e.g. a row of "Revoke"). */
    ariaLabel?: string;
    children?: Snippet;
  }

  let {
    variant = 'primary',
    size = 'md',
    type = 'button',
    disabled = false,
    onclick,
    ariaLabel,
    children,
  }: Props = $props();
</script>

<button {type} {disabled} {onclick} aria-label={ariaLabel} class="btn {variant} {size}">
  {#if children}{@render children()}{/if}
</button>

<style>
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    cursor: pointer;
    font-size: var(--text-sm);
    font-weight: 500;
    transition: background 0.1s, color 0.1s, opacity 0.1s, border-color 0.1s;
    white-space: nowrap;
  }

  .btn:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  /* Sizes */
  .sm { padding: 0.25rem 0.625rem; font-size: var(--text-xs); }
  .md { padding: 0.4375rem 0.875rem; }
  .lg { padding: 0.625rem 1.25rem; font-size: var(--text-md); }

  /* Variants */
  .primary {
    background: var(--accent);
    color: var(--bg-base);
    font-weight: 600;
  }
  .primary:not(:disabled):hover { background: var(--accent-hover); }
  .primary:not(:disabled):active { background: var(--accent-active); }

  .ghost {
    background: transparent;
    color: var(--text-2);
  }
  .ghost:not(:disabled):hover { background: var(--bg-hover); color: var(--text-1); }

  .danger {
    background: var(--danger);
    color: var(--bg-base);
    font-weight: 600;
  }
  .danger:not(:disabled):hover { opacity: 0.85; }

  .outline {
    background: transparent;
    color: var(--text-2);
    border-color: var(--border-strong);
  }
  .outline:not(:disabled):hover { border-color: var(--accent); color: var(--text-1); }
</style>
