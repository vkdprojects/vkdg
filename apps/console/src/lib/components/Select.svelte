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

  let inputId = $state(id ?? `select-${Math.random().toString(36).slice(2)}`);
  let errorId = $derived(`${inputId}-error`);

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
    <Select.Trigger id={inputId} class="select-trigger" aria-describedby={error ? errorId : undefined}>
      <span class="select-value" class:placeholder={!value}>
        {value ? selectedLabel : placeholder}
      </span>
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
        <polyline points="6 9 12 15 18 9"/>
      </svg>
    </Select.Trigger>

    <Select.Content class="select-content" sideOffset={4}>
      {#each options as opt}
        <Select.Item value={opt.value} class="select-item">
          {opt.label}
        </Select.Item>
      {/each}
    </Select.Content>
  </Select.Root>

  {#if error}
    <p id={errorId} class="error" role="alert">{error}</p>
  {/if}
</div>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: 0.375rem;
  }

  label {
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--text-2);
  }

  :global(.select-trigger) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--text-1);
    cursor: pointer;
    font-size: 0.875rem;
    padding: 0.4375rem 0.625rem;
    transition: border-color 0.15s;
    width: 100%;
    text-align: left;
  }

  :global(.select-trigger:focus) {
    border-color: var(--accent);
    outline: none;
  }

  :global(.select-trigger[data-disabled]) {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .select-value.placeholder {
    color: var(--text-3);
  }

  :global(.select-content) {
    background: var(--bg-elevated);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
    min-width: 160px;
    padding: 4px;
    z-index: 50;
  }

  :global(.select-item) {
    border-radius: var(--radius-sm);
    color: var(--text-2);
    cursor: pointer;
    font-size: 0.875rem;
    padding: 0.375rem 0.625rem;
    transition: background 0.1s, color 0.1s;
  }

  :global(.select-item:hover),
  :global(.select-item[data-highlighted]) {
    background: var(--bg-hover);
    color: var(--text-1);
    outline: none;
  }

  :global(.select-item[data-selected]) {
    color: var(--accent);
  }

  .error {
    font-size: 0.75rem;
    color: var(--danger);
    margin: 0;
  }
</style>
