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
    /** Helper text under the field, linked to it with aria-describedby. */
    hint?: string;
    /** Number inputs only. */
    min?: number;
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
    hint,
    min,
    autocomplete,
  }: Props = $props();

  const inputId = $derived(id ?? `input-${Math.random().toString(36).slice(2)}`);
  const errorId = $derived(`${inputId}-error`);
  const hintId = $derived(`${inputId}-hint`);
  const describedBy = $derived(error ? errorId : hint ? hintId : undefined);
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
    {min}
    aria-invalid={error ? 'true' : undefined}
    aria-describedby={describedBy}
    class:has-error={!!error}
  />
  {#if error}
    <p id={errorId} class="error-msg" role="alert">{error}</p>
  {:else if hint}
    <p id={hintId} class="field-hint">{hint}</p>
  {/if}
</div>

<style>
  .field { min-width: 0; }

  /* Base label is a column flexbox; keep the required marker inline with the text. */
  label { display: block; }

  .req { color: var(--danger); }

  input.has-error,
  input.has-error:hover:not(:focus) {
    border-color: var(--danger);
  }

  input.has-error:focus {
    box-shadow: 0 0 0 var(--focus-w) var(--danger-subtle);
  }

  .error-msg { margin: 0; }
  .field-hint { margin: 0; }
</style>
