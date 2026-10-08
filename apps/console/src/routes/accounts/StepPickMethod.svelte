<script lang="ts">
  import { Check } from 'lucide-svelte';
  import type { LoginMethod } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { Button, Select, Spinner } from '$lib/components/index.js';

  interface Props {
    providers: { value: string; label: string }[];
    selected: string;
    methods: LoginMethod[];
    methodId: string;
    /** The currently chosen method; `undefined` until methods load. */
    method: LoginMethod | undefined;
    params: Record<string, string>;
    loadingMethods: boolean;
    busy: boolean;
    onpick: (id: string) => void;
    onsubmit: (e: Event) => void;
    oncancel: () => void;
  }

  let {
    providers,
    selected = $bindable(),
    methods,
    methodId,
    method,
    params = $bindable(),
    loadingMethods,
    busy,
    onpick,
    onsubmit,
    oncancel,
  }: Props = $props();
</script>

<form {onsubmit} class="fields">
  <Select label={m.acct_provider()} options={providers} bind:value={selected} disabled={busy} />

  {#if loadingMethods}
    <div class="muted status-row"><Spinner size="sm" /> {m.common_loading()}</div>
  {:else if methods.length === 0}
    <p class="muted status-row">{m.acct_no_methods()}</p>
  {:else}
    {#if methods.length > 1}
      <!-- Cards per método: mais claro que um dropdown quando >1 opção -->
      <div class="method-grid">
        {#each methods as mx (mx.id)}
          <button
            type="button"
            class="method-card"
            class:selected={methodId === mx.id}
            onclick={() => onpick(mx.id)}
            aria-pressed={methodId === mx.id}
          >
            {#if methodId === mx.id}
              <Check class="method-check" size={14} aria-hidden="true" />
            {/if}
            {#if mx.icon_char}
              <span class="method-icon">{mx.icon_char}</span>
            {/if}
            <span class="method-label">{mx.label}</span>
            {#if mx.hint}
              <span class="method-hint">{mx.hint}</span>
            {/if}
          </button>
        {/each}
      </div>
    {/if}
    {#each method?.fields ?? [] as f (f.id)}
      {#if f.id === 'provider' && method?.id === 'social'}
        <!-- Kiro social: Google vs GitHub como radio, não campo livre -->
        <fieldset class="social-choice">
          <legend class="field-label">{f.label}</legend>
          <div class="social-options">
            {#each ['Google', 'Github'] as opt (opt)}
              <label class="social-opt" class:checked={params[f.id] === opt}>
                <input
                  type="radio"
                  name="social-provider"
                  value={opt}
                  bind:group={params[f.id]}
                />
                {#if params[f.id] === opt}
                  <Check class="social-check" size={12} aria-hidden="true" />
                {/if}
                {opt}
              </label>
            {/each}
          </div>
        </fieldset>
      {:else}
        <div class="field">
          <label for="login-{f.id}">{f.label}{#if f.required}<span aria-hidden="true"> *</span>{/if}</label>
          <input
            id="login-{f.id}"
            type={f.secret ? 'password' : 'text'}
            autocomplete="off"
            required={f.required}
            placeholder={f.placeholder ?? ''}
            bind:value={params[f.id]}
          />
        </div>
      {/if}
    {/each}
  {/if}
  <div class="dialog-footer">
    <Button variant="outline" onclick={oncancel}>{m.common_cancel()}</Button>
    <Button type="submit" disabled={busy || !method}>
      {#if busy}<Spinner size="sm" />{/if}
      {m.acct_start()}
    </Button>
  </div>
</form>

<style>
  .method-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, var(--col-sm)), 1fr));
    gap: var(--space-2);
  }

  .method-card {
    position: relative;
    align-items: flex-start;
    background: var(--bg-elevated);
    border: var(--border-w) solid var(--border);
    border-radius: var(--radius);
    color: inherit;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-height: var(--control-h-lg);
    padding: var(--space-3) var(--space-4);
    text-align: left;
    width: 100%;
  }

  .method-card:hover {
    border-color: var(--accent);
  }

  .method-card.selected {
    border-color: var(--accent);
    background: var(--accent-subtle);
    box-shadow: 0 0 0 var(--border-w) var(--accent);
  }

  :global(.method-check) {
    position: absolute;
    top: var(--space-2);
    right: var(--space-2);
    color: var(--accent);
  }

  .method-icon {
    font-size: var(--text-lg);
    line-height: 1;
  }

  .method-label {
    color: var(--text-1);
    font-size: var(--text-base);
    font-weight: var(--weight-semibold);
    padding-right: var(--space-4);
  }

  .method-hint {
    color: var(--text-3);
    font-size: var(--text-xs);
    line-height: var(--leading);
  }

  .social-choice {
    border: none;
    margin: 0;
    padding: 0;
    min-width: 0;
  }

  .social-choice .field-label {
    margin-bottom: var(--space-2);
    display: block;
    padding: 0;
  }

  .social-options {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .social-opt {
    position: relative;
    flex-direction: row;
    align-items: center;
    background: var(--bg-elevated);
    border: var(--border-w) solid var(--border);
    border-radius: var(--radius);
    cursor: pointer;
    display: flex;
    gap: var(--space-2);
    min-height: var(--control-h);
    padding: 0 var(--space-4);
    font-size: var(--text-base);
    font-weight: var(--weight-medium);
    color: var(--text-2);
  }

  .social-opt input[type='radio'] {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }

  .social-opt:hover { border-color: var(--accent); }

  .social-opt:has(input:focus-visible) {
    outline: var(--focus-w) solid var(--accent);
    outline-offset: var(--focus-w);
  }

  .social-opt.checked {
    border-color: var(--accent);
    background: var(--accent-subtle);
    color: var(--text-1);
  }

  :global(.social-check) {
    color: var(--accent);
  }
</style>
