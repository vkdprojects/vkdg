<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api.js';
  import type { ComboSummary } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { Badge, EmptyState, Button, Select, Spinner } from '$lib/components/index.js';
  import { toast } from 'svelte-sonner';

  let combos = $state<ComboSummary[]>([]);
  let loading = $state(true);
  let formError = $state('');

  let dialogEl = $state<HTMLDialogElement | null>(null);
  let name = $state('');
  let patterns = $state('');
  let strategy = $state('round_robin');
  let targets = $state('');

  function openDialog() {
    name = '';
    patterns = '';
    strategy = 'round_robin';
    targets = '';
    dialogEl?.showModal();
  }

  function closeDialog() {
    dialogEl?.close();
  }

  const strategyOptions = [
    { value: 'round_robin', label: m.combo_strategy_round_robin() },
    { value: 'weighted', label: m.combo_strategy_weighted() },
    { value: 'fallback_chain', label: m.combo_strategy_fallback_chain() },
    { value: 'lowest_latency', label: m.combo_strategy_lowest_latency() },
    { value: 'power_of_two_choices', label: m.combo_strategy_power_of_two_choices() },
    { value: 'last_known_good', label: m.combo_strategy_last_known_good() },
    { value: 'fusion', label: m.combo_strategy_fusion() },
    { value: 'prompt_chain', label: m.combo_strategy_prompt_chain() },
    { value: 'auto', label: m.combo_strategy_auto() },
  ];

  onMount(async () => {
    try {
      const res = await api.listCombos();
      combos = res.items;
    } catch (e) {
      toast.error((e as Error).message);
    } finally {
      loading = false;
    }
  });

  async function createCombo(e: Event) {
    e.preventDefault();
    formError = 'Not yet implemented';
    toast.info('Combo creation via UI is coming in the next release. Use vkdg.yaml for now.');
    closeDialog();
  }
</script>

<div class="page">
  <div class="page-header">
    <h1 class="page-title">{m.nav_combos()} ({combos.length})</h1>
    <Button onclick={openDialog}>{m.combo_create()}</Button>
  </div>

  {#if loading}
    <div class="loading"><Spinner size="sm" /> Loading…</div>
  {:else if combos.length === 0}
    <EmptyState title={m.combo_empty()} description="Create a combo to define routing strategy across connections." />
  {:else}
    <table>
      <thead>
        <tr>
          <th scope="col">{m.combo_id()}</th>
          <th scope="col">{m.combo_strategy()}</th>
          <th scope="col">{m.combo_patterns()}</th>
          <th scope="col">{m.combo_targets()}</th>
          <th scope="col">{m.combo_policies()}</th>
        </tr>
      </thead>
      <tbody>
        {#each combos as c (c.id)}
          <tr>
            <td>{c.id}</td>
            <td>{c.strategy}</td>
            <td>{c.match_patterns.join(', ') || m.common_none()}</td>
            <td>{c.targets.join(', ') || m.common_none()}</td>
            <td class="badges">
              {#if c.has_compression}<Badge status="compression" label="compression" />{/if}
              {#if c.has_cache}<Badge status="cache" label="cache" />{/if}
              {#if c.has_budget}<Badge status="budget" label="budget" />{/if}
              {#if !c.has_compression && !c.has_cache && !c.has_budget}<span class="muted">{m.common_none()}</span>{/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

<dialog bind:this={dialogEl} class="modal" aria-labelledby="dialog-title">
  <div class="modal-header">
    <h2 id="dialog-title">{m.combo_create()}</h2>
    <button class="close-btn" onclick={closeDialog} aria-label={m.common_cancel()}>✕</button>
  </div>

  <form onsubmit={createCombo} class="modal-form">
    <div class="field">
      <label for="name-input">{m.combo_name_label()}</label>
      <input
        id="name-input"
        type="text"
        bind:value={name}
        placeholder={m.combo_name_placeholder()}
        required
      />
    </div>

    <div class="field">
      <label for="patterns-input">{m.combo_patterns_label()}</label>
      <input
        id="patterns-input"
        type="text"
        bind:value={patterns}
        placeholder={m.combo_patterns_placeholder()}
      />
      <p class="hint">{m.combo_patterns_hint()}</p>
    </div>

    <Select
      label={m.combo_strategy()}
      options={strategyOptions}
      bind:value={strategy}
    />

    <div class="field">
      <label for="targets-input">{m.combo_targets_label()}</label>
      <input
        id="targets-input"
        type="text"
        bind:value={targets}
        placeholder={m.combo_targets_placeholder()}
      />
      <p class="hint">{m.combo_targets_hint()}</p>
    </div>

    {#if formError}
      <p class="form-error" role="alert">{formError}</p>
    {/if}

    <div class="modal-actions">
      <Button variant="ghost" type="button" onclick={closeDialog}>{m.common_cancel()}</Button>
      <Button type="submit">{m.combo_create()}</Button>
    </div>
  </form>
</dialog>

<style>
  .loading {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-3);
    font-size: 0.875rem;
    padding: 16px 0;
  }

  .badges {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }

  .muted {
    color: var(--text-3);
  }

  .modal {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text-1);
    padding: 0;
    width: min(480px, calc(100vw - 32px));
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.32);
  }

  .modal::backdrop {
    background: rgba(0, 0, 0, 0.5);
  }

  .modal-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 20px 24px 0;
  }

  .modal-header h2 {
    font-size: 1rem;
    font-weight: 600;
    margin: 0;
  }

  .close-btn {
    background: transparent;
    border: none;
    color: var(--text-3);
    cursor: pointer;
    font-size: 1rem;
    line-height: 1;
    padding: 4px;
    border-radius: var(--radius-sm);
    transition: color 0.1s;
  }

  .close-btn:hover {
    color: var(--text-1);
  }

  .modal-form {
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 20px 24px 24px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 0.375rem;
  }

  .field label {
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--text-2);
  }

  .field input {
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--text-1);
    font-size: 0.875rem;
    padding: 0.4375rem 0.625rem;
    transition: border-color 0.15s;
    width: 100%;
    box-sizing: border-box;
  }

  .field input:focus {
    border-color: var(--accent);
    outline: none;
  }

  .field input::placeholder {
    color: var(--text-3);
  }

  .hint {
    font-size: 0.75rem;
    color: var(--text-3);
    margin: 0;
  }

  .form-error {
    font-size: 0.8125rem;
    color: var(--danger);
    margin: 0;
  }

  .modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }
</style>
