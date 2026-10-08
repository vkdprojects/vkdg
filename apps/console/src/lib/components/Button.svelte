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
    display: inline-flex; align-items: center; justify-content: center; gap: var(--space-2);
    border: 1px solid transparent; border-radius: var(--radius);
    cursor: pointer; font: inherit; font-size: var(--text-sm); font-weight: var(--weight-medium);
    white-space: nowrap; user-select: none;
    line-height: 1; vertical-align: middle; flex-shrink: 0;
  }
  .btn :global(svg) { flex-shrink: 0; width: var(--icon-sm); height: var(--icon-sm);
  }
  .btn:not(:disabled):active { transform: var(--press); }
  .btn:disabled { opacity: 0.45; cursor: not-allowed; }
  .sm { height: var(--control-h-sm); padding: 0 var(--space-3); font-size: var(--text-xs); border-radius: var(--radius-sm); }
  .md { height: var(--control-h); padding: 0 var(--control-px); }
  .lg { height: var(--control-h-lg); padding: 0 var(--space-5); font-size: var(--text-md); }

  .primary {
    background: var(--accent); color: var(--on-accent); font-weight: var(--weight-semibold);
    box-shadow: var(--shadow-1);
  }
  .primary:not(:disabled):hover { background: var(--accent-hover); box-shadow: var(--glow); }
  .ghost { background: transparent; color: var(--text-2); }
  .ghost:not(:disabled):hover { background: var(--bg-hover); color: var(--text-1); }
  .danger { background: var(--danger-subtle); color: var(--danger); border-color: color-mix(in oklch, var(--danger) 35%, transparent); }
  .danger:not(:disabled):hover { background: var(--danger); color: var(--on-accent); }
  .outline { background: var(--bg-surface); color: var(--text-1); border-color: var(--border-strong); box-shadow: var(--shadow-1); }
  .outline:not(:disabled):hover { border-color: var(--accent); background: var(--accent-subtle); }
</style>
