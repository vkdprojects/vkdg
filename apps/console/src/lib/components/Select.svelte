<script lang="ts">
  import { Select } from 'bits-ui';
  import type { Snippet } from 'svelte';

  interface SelectOption {
    value: string;
    label: string;
  }

  interface Props {
    id?: string;
    label?: string;
    options: SelectOption[];
    value?: string;
    placeholder?: string;
    disabled?: boolean;
    error?: string;
    onchange?: (value: string) => void;
  }

  let {
    id,
    label: labelText,
    options,
    value = $bindable(''),
    placeholder = 'Select…',
    disabled = false,
    error,
    onchange,
  }: Props = $props();

  const inputId = $derived(id ?? `select-${Math.random().toString(36).slice(2)}`);
  const errorId = $derived(`${inputId}-error`);

  const selectedLabel = $derived(options.find((o) => o.value === value)?.label ?? '');
</script>

<div class="field">
  {#if labelText}
    <label for={inputId}>{labelText}</label>
  {/if}

  <Select.Root
    type="single"
    bind:value
    onValueChange={(v) => onchange?.(v)}
    {disabled}
  >
    <Select.Trigger id={inputId} class="select-trigger" aria-describedby={error ? errorId : undefined} aria-invalid={error ? 'true' : undefined}>
      <span class="select-value" class:placeholder={!value}>
        {value ? selectedLabel : placeholder}
      </span>
      <svg class="chevron" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <polyline points="6 9 12 15 18 9"/>
      </svg>
    </Select.Trigger>

    <Select.Content class="select-content" sideOffset={6}>
      {#each options as opt}
        <!-- `label` drives bits-ui typeahead: without it, typing selects nothing. -->
        <Select.Item value={opt.value} label={opt.label} class="select-item">
          {opt.label}
        </Select.Item>
      {/each}
    </Select.Content>
  </Select.Root>

  {#if error}
    <p id={errorId} class="error-msg" role="alert">{error}</p>
  {/if}
</div>

<style>
  .field { min-width: 0; }

  label { display: block; }

  :global(.select-trigger) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    background: var(--bg-inset);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text-1);
    cursor: pointer;
    font: inherit;
    font-size: var(--text-base);
    min-height: var(--control-h);
    padding: var(--space-2) var(--control-px);
    width: 100%;
    text-align: left;
  }

  :global(.select-trigger:hover:not(:focus):not([data-disabled])) {
    border-color: var(--border-strong);
  }

  :global(.select-trigger:focus),
  :global(.select-trigger[data-state='open']) {
    border-color: var(--accent);
    box-shadow: var(--ring);
    outline: none;
  }

  :global(.select-trigger[aria-invalid='true']) {
    border-color: var(--danger);
  }

  :global(.select-trigger[data-disabled]) {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .select-value {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .select-value.placeholder {
    color: var(--text-3);
  }

  .chevron {
    flex-shrink: 0;
    color: var(--text-3);
    transition: transform var(--dur-2) var(--ease-out);
  }

  :global(.select-trigger[data-state='open']) .chevron {
    transform: rotate(180deg);
    color: var(--accent);
  }

  :global(.select-content) {
    background: var(--bg-elevated);
    border: var(--border-w) solid var(--border-strong);
    border-radius: var(--radius);
    box-shadow: var(--shadow-2);
    min-width: max(var(--col-sm), var(--bits-select-anchor-width, 0px)); /* token-ok */
    max-height: min(var(--col-lg), var(--bits-select-content-available-height, var(--col-lg)));
    overflow-y: auto;
    padding: var(--space-1);
    z-index: var(--z-dialog);
    animation: select-in var(--dur-2) var(--ease-out);
  }

  :global(.select-item) {
    border-radius: var(--radius-sm);
    color: var(--text-2);
    cursor: pointer;
    font-size: var(--text-base);
    min-height: var(--control-h-sm);
    display: flex;
    align-items: center;
    padding: var(--space-1) var(--space-3);
    transition: background var(--dur-1) ease, color var(--dur-1) ease;
  }

  :global(.select-item:hover),
  :global(.select-item[data-highlighted]) {
    background: var(--bg-hover);
    color: var(--text-1);
    outline: none;
  }

  :global(.select-item[data-selected]) {
    background: var(--accent-subtle);
    color: var(--accent);
    font-weight: var(--weight-medium);
  }

  .error-msg { margin: 0; }

  @keyframes select-in {
    from { opacity: 0; transform: translateY(calc(var(--space-1) * -1)) scale(0.98); }
  }
</style>
