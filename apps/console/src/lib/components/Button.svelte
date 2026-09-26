<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    variant?: 'primary' | 'ghost' | 'danger' | 'outline';
    size?: 'sm' | 'md' | 'lg';
    type?: 'button' | 'submit' | 'reset';
    disabled?: boolean;
    onclick?: () => void;
    children?: Snippet;
  }

  let {
    variant = 'primary',
    size = 'md',
    type = 'button',
    disabled = false,
    onclick,
    children,
  }: Props = $props();
</script>

<button {type} {disabled} {onclick} class="btn {variant} {size}">
  {#if children}{@render children()}{/if}
</button>

<style>
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    border: none;
    border-radius: var(--radius-sm);
    cursor: pointer;
    font-size: 0.8125rem;
    font-weight: 500;
    transition: background 0.1s, color 0.1s, opacity 0.1s;
    white-space: nowrap;
  }

  .btn:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  /* Sizes */
  .sm { padding: 0.25rem 0.625rem; font-size: 0.75rem; }
  .md { padding: 0.4375rem 0.875rem; }
  .lg { padding: 0.625rem 1.25rem; font-size: 0.9375rem; }

  /* Variants */
  .primary {
    background: var(--accent);
    color: #fff;
  }
  .primary:not(:disabled):hover { background: var(--accent-hover); }

  .ghost {
    background: transparent;
    color: var(--text-2);
  }
  .ghost:not(:disabled):hover { background: var(--bg-hover); color: var(--text-1); }

  .danger {
    background: var(--danger);
    color: #fff;
  }
  .danger:not(:disabled):hover { opacity: 0.85; }

  .outline {
    background: transparent;
    color: var(--text-2);
    border: 1px solid var(--border-strong);
  }
  .outline:not(:disabled):hover { border-color: var(--text-3); color: var(--text-1); }
</style>
