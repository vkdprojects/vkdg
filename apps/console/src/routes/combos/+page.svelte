<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api.js';
  import type { ComboInput, ComboStrategy, ComboSummary, ConnectionSummary } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { Badge, EmptyState, Button, Select, Spinner } from '$lib/components/index.js';
  import { toast } from 'svelte-sonner';

  let combos = $state<ComboSummary[]>([]);
  let connections = $state<ConnectionSummary[]>([]);
  let loading = $state(true);
  let formError = $state('');
  let saving = $state(false);

  let dialogEl = $state<HTMLDialogElement | null>(null);
  /** Id of the combo being edited; null = creating. */
  let editingId = $state<string | null>(null);
  let name = $state('');
  let patterns = $state('');
  let strategy = $state('round_robin');
  let targets = $state<string[]>([]);
  let model = $state('');
  let maxCostUsd = $state<number | null>(null);

  // Only what the router runs; anything else would be refused by the API.
  const strategyOptions = [
    { value: 'round_robin', label: m.combo_strategy_round_robin() },
    { value: 'fallback_chain', label: m.combo_strategy_fallback_chain() },
    { value: 'lowest_latency', label: m.combo_strategy_lowest_latency() },
    { value: 'power_of_two_choices', label: m.combo_strategy_power_of_two_choices() },
    { value: 'fusion', label: m.combo_strategy_fusion() },
  ];

  function strategyName(s: ComboSummary['strategy']): string {
    return typeof s === 'string' ? s : Object.keys(s)[0] ?? 'unknown';
  }

  function strategyValue(v: string): ComboStrategy {
    return v === 'fusion' ? { fusion: { max_candidates: null } } : (v as ComboStrategy);
  }

  const splitList = (v: string) => v.split(',').map((x) => x.trim()).filter(Boolean);

  async function refresh() {
    const res = await api.listCombos();
    combos = res.items;
  }

  onMount(async () => {
    try {
      const [, conns] = await Promise.all([refresh(), api.listConnections()]);
      connections = conns.items;
    } catch (e) {
      toast.error((e as Error).message);
    } finally {
      loading = false;
    }
  });

  function openCreate() {
    editingId = null;
    name = '';
    patterns = '';
    strategy = 'round_robin';
    targets = [];
    model = '';
    maxCostUsd = null;
    formError = '';
    dialogEl?.showModal();
  }

  function openEdit(c: ComboSummary) {
    editingId = c.id;
    name = c.id;
    patterns = c.match_patterns.join(', ');
    strategy = strategyName(c.strategy);
    targets = [...c.targets];
    model = c.model ?? '';
    maxCostUsd = c.max_cost_microdollars != null ? c.max_cost_microdollars / 1_000_000 : null;
    formError = '';
    dialogEl?.showModal();
  }

  function closeDialog() {
    dialogEl?.close();
  }

  async function save(e: Event) {
    e.preventDefault();
    formError = '';
    if (!name.trim()) { formError = m.combo_name_required(); return; }
    if (targets.length === 0) { formError = m.combo_targets_required(); return; }
    if (!model.trim()) { formError = m.combo_model_required(); return; }
    const body: ComboInput = {
      match_patterns: splitList(patterns),
      strategy: strategyValue(strategy),
      targets,
      model: model.trim(),
    };
    if (maxCostUsd != null) body.max_cost_microdollars = Math.round(maxCostUsd * 1_000_000);
    saving = true;
    try {
      if (editingId) {
        await api.updateCombo(editingId, body);
      } else {
        await api.createCombo({ ...body, id: name.trim() });
      }
      await refresh();
      closeDialog();
      toast.success(m.combo_saved());
    } catch (err) {
      // Backend 400/409 (unknown target, bad id, duplicate) shows in the form.
      formError = (err as Error).message;
    } finally {
      saving = false;
    }
  }

  async function remove(c: ComboSummary) {
    if (!confirm(m.combo_delete_confirm({ id: c.id }))) return;
    try {
      await api.deleteCombo(c.id);
      await refresh();
    } catch (err) {
      toast.error((err as Error).message);
    }
  }
</script>

<div class="page">
  <div class="page-header">
    <h1 class="page-title">{m.nav_combos()} ({combos.length})</h1>
    <Button onclick={openCreate}>{m.combo_create()}</Button>
  </div>

  {#if loading}
    <div class="loading"><Spinner size="sm" /> Loading…</div>
  {:else if combos.length === 0}
    <EmptyState title={m.combo_empty()} description={m.combo_empty_desc()} />
  {:else}
    <table>
      <thead>
        <tr>
          <th scope="col">{m.combo_id()}</th>
          <th scope="col">{m.combo_strategy()}</th>
          <th scope="col">{m.combo_patterns()}</th>
          <th scope="col">{m.combo_targets()}</th>
          <th scope="col">{m.combo_model()}</th>
          <th scope="col">{m.combo_policies()}</th>
          <th scope="col"><span class="sr-only">{m.combo_actions()}</span></th>
        </tr>
      </thead>
      <tbody>
        {#each combos as c (c.id)}
          <tr>
            <td>{c.id}</td>
            <td>{strategyName(c.strategy)}</td>
            <td>{c.match_patterns.join(', ') || m.common_none()}</td>
            <td>{c.targets.join(', ') || m.common_none()}</td>
            <td class="mono">{c.model ?? m.common_none()}</td>
            <td class="badges">
              {#if c.has_compression}<Badge status="compression" label="compression" />{/if}
              {#if c.has_cache}<Badge status="cache" label="cache" />{/if}
              {#if c.has_budget}<Badge status="budget" label="budget" />{/if}
              {#if !c.has_compression && !c.has_cache && !c.has_budget}<span class="muted">{m.common_none()}</span>{/if}
            </td>
            <td class="actions">
              <Button size="sm" variant="outline" onclick={() => openEdit(c)} ariaLabel={m.combo_edit_label({ id: c.id })}>{m.common_edit()}</Button>
              <Button size="sm" variant="ghost" onclick={() => remove(c)} ariaLabel={m.combo_delete_label({ id: c.id })}>{m.common_delete()}</Button>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

<dialog bind:this={dialogEl} class="modal" aria-labelledby="dialog-title">
  <div class="modal-header">
    <h2 id="dialog-title">{editingId ? m.combo_edit_title({ id: editingId }) : m.combo_create()}</h2>
    <button class="close-btn" onclick={closeDialog} aria-label={m.common_cancel()}>✕</button>
  </div>

  <form onsubmit={save} class="modal-form">
    <div class="field">
      <label for="name-input">{m.combo_name_label()}</label>
      <input
        id="name-input"
        type="text"
        bind:value={name}
        placeholder={m.combo_name_placeholder()}
        required
        disabled={editingId !== null}
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

    <fieldset class="field">
      <legend>{m.combo_targets_label()}</legend>
      {#if connections.length === 0}
        <p class="hint">{m.combo_no_connections()}</p>
      {/if}
      {#each connections as conn (conn.id)}
        <label class="check-row">
          <input type="checkbox" value={conn.id} bind:group={targets} />
          <span class="mono">{conn.id}</span>
          <span class="muted">{conn.provider}</span>
        </label>
      {/each}
      <p class="hint">{m.combo_targets_hint()}</p>
    </fieldset>

    <div class="field">
      <label for="model-input">{m.combo_model()}</label>
      <input id="model-input" type="text" bind:value={model} placeholder="claude-sonnet-4-5" required aria-describedby="model-hint" />
      <p id="model-hint" class="hint">{m.combo_model_hint()}</p>
    </div>

    <div class="field">
      <label for="budget-input">{m.combo_budget_label()}</label>
      <input id="budget-input" type="number" min="0" step="0.000001" bind:value={maxCostUsd} aria-describedby="budget-hint" />
      <p id="budget-hint" class="hint">{m.combo_budget_hint()}</p>
    </div>

    {#if formError}
      <p class="form-error" role="alert">{formError}</p>
    {/if}

    <div class="modal-actions">
      <Button variant="ghost" type="button" onclick={closeDialog}>{m.common_cancel()}</Button>
      <Button type="submit" disabled={saving}>{editingId ? m.common_save() : m.combo_create()}</Button>
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

  .actions {
    display: flex;
    gap: 4px;
    justify-content: flex-end;
  }

  .check-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 0.875rem;
  }

  fieldset.field {
    border: none;
    padding: 0;
    margin: 0;
  }

  fieldset.field legend {
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--text-2);
    margin-bottom: 0.375rem;
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
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
