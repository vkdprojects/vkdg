<script lang="ts">
  interface Props {
    id?: string;
    label?: string;
    type?: 'text' | 'password' | 'email' | 'number';
    value?: string;
    placeholder?: string;
    required?: boolean;
    disabled?: boolean;
    error?: string;
    autocomplete?: HTMLInputElement['autocomplete'];
  }

  let {
    id,
    label,
    type = 'text',
    value = $bindable(''),
    placeholder,
    required = false,
    disabled = false,
    error,
    autocomplete,
  }: Props = $props();

  let inputId = $state(id ?? `input-${Math.random().toString(36).slice(2)}`);
  let errorId = $derived(`${inputId}-error`);
</script>

<div class="field">
  {#if label}
    <label for={inputId}>{label}{#if required}<span class="req" aria-hidden="true"> *</span>{/if}</label>
  {/if}
  <input
    id={inputId}
    {type}
    bind:value
    {placeholder}
    {required}
    {disabled}
    {autocomplete}
    aria-invalid={error ? 'true' : undefined}
    aria-describedby={error ? errorId : undefined}
    class:has-error={!!error}
  />
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

  .req {
    color: var(--danger);
  }

  input {
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--text-1);
    font-size: 0.875rem;
    padding: 0.4375rem 0.625rem;
    transition: border-color 0.15s;
    width: 100%;
  }

  input:focus {
    border-color: var(--accent);
    outline: none;
  }

  input::placeholder {
    color: var(--text-3);
  }

  input:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  input.has-error {
    border-color: var(--danger);
  }

  .error {
    font-size: 0.75rem;
    color: var(--danger);
    margin: 0;
  }
</style>
