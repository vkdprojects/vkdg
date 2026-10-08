<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api.js';
  import type { ComboInput, ComboStrategy, ComboSummary, ConnectionSummary } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { Badge, EmptyState, Button, Select } from '$lib/components/index.js';
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
    <div>
      <h1>{m.nav_combos()} <span class="count mono">{combos.length}</span></h1>
    </div>
    <div class="page-actions">
      <Button onclick={openCreate}>{m.combo_create()}</Button>
    </div>
  </div>

  {#if loading}
    <section class="panel" aria-busy="true">
      <div class="panel-body loading">
        <div class="skeleton row"></div>
        <div class="skeleton row"></div>
        <div class="skeleton row"></div>
      </div>
    </section>
  {:else if combos.length === 0}
    <EmptyState title={m.combo_empty()} description={m.combo_empty_desc()} />
  {:else}
    <section class="panel">
      <div class="table-wrap">
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
                <td class="id">{c.id}</td>
                <td><span class="strategy-chip mono">{strategyName(c.strategy)}</span></td>
                <td>{c.match_patterns.join(', ') || m.common_none()}</td>
                <td>{c.targets.join(', ') || m.common_none()}</td>
                <td class="mono">{c.model ?? m.common_none()}</td>
                <td>
                  <div class="badges">
                    {#if c.has_compression}<Badge status="compression" label="compression" />{/if}
                    {#if c.has_cache}<Badge status="cache" label="cache" />{/if}
                    {#if c.has_budget}<Badge status="budget" label="budget" />{/if}
                    {#if !c.has_compression && !c.has_cache && !c.has_budget}<span class="muted">{m.common_none()}</span>{/if}
                  </div>
                </td>
                <td>
                  <div class="actions">
                    <Button size="sm" variant="outline" onclick={() => openEdit(c)} ariaLabel={m.combo_edit_label({ id: c.id })}>{m.common_edit()}</Button>
                    <Button size="sm" variant="ghost" onclick={() => remove(c)} ariaLabel={m.combo_delete_label({ id: c.id })}>{m.common_delete()}</Button>
                  </div>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    </section>
  {/if}
</div>

<dialog bind:this={dialogEl} class="dialog-content" aria-labelledby="dialog-title">
  <div class="dialog-header">
    <h2 id="dialog-title" class="dialog-title">{editingId ? m.combo_edit_title({ id: editingId }) : m.combo_create()}</h2>
    <button class="dialog-close" onclick={closeDialog} aria-label={m.common_cancel()}>✕</button>
  </div>

  <form onsubmit={save}>
   <div class="dialog-body fields">
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
      <p class="field-hint">{m.combo_patterns_hint()}</p>
    </div>

    <Select
      label={m.combo_strategy()}
      options={strategyOptions}
      bind:value={strategy}
    />

    <fieldset class="field">
      <legend>{m.combo_targets_label()}</legend>
      {#if connections.length === 0}
        <p class="field-hint">{m.combo_no_connections()}</p>
      {/if}
      {#each connections as conn (conn.id)}
        <label class="check-row">
          <input type="checkbox" value={conn.id} bind:group={targets} />
          <span class="mono">{conn.id}</span>
          <span class="muted">{conn.provider}</span>
        </label>
      {/each}
      <p class="field-hint">{m.combo_targets_hint()}</p>
    </fieldset>

    <div class="field">
      <label for="model-input">{m.combo_model()}</label>
      <input id="model-input" type="text" bind:value={model} placeholder="claude-sonnet-4-5" required aria-describedby="model-hint" />
      <p id="model-hint" class="field-hint">{m.combo_model_hint()}</p>
    </div>

    <div class="field">
      <label for="budget-input">{m.combo_budget_label()}</label>
      <input id="budget-input" type="number" min="0" step="0.000001" bind:value={maxCostUsd} aria-describedby="budget-hint" />
      <p id="budget-hint" class="field-hint">{m.combo_budget_hint()}</p>
    </div>

    {#if formError}
      <p class="form-error" role="alert">{formError}</p>
    {/if}
   </div>

    <div class="dialog-footer">
      <Button variant="ghost" type="button" onclick={closeDialog}>{m.common_cancel()}</Button>
      <Button type="submit" disabled={saving}>{editingId ? m.common_save() : m.combo_create()}</Button>
    </div>
  </form>
</dialog>

<style>
  .count {
    color: var(--text-3);
    font-weight: var(--weight-regular);
    font-size: var(--text-md);
    margin-left: var(--space-1);
  }

  .loading {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .skeleton.row {
    height: var(--control-h);
  }

  .id {
    color: var(--text-1);
    font-weight: var(--weight-medium);
    white-space: nowrap;
  }

  .strategy-chip {
    display: inline-block;
    padding: var(--space-0) var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--accent-subtle);
    color: var(--accent);
    white-space: nowrap;
  }

  .badges {
    display: flex;
    gap: var(--space-1);
    flex-wrap: wrap;
  }

  .actions {
    display: flex;
    gap: var(--space-1);
    justify-content: flex-end;
  }

  .check-row {
    flex-direction: row;
    align-items: center;
    gap: var(--space-2);
    min-height: var(--control-h);
    font-weight: var(--weight-regular);
    cursor: pointer;
  }

  fieldset.field {
    border: none;
    padding: 0;
    margin: 0;
    min-width: 0;
  }

  fieldset.field legend {
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    color: var(--text-2);
    margin-bottom: var(--space-2);
    padding: 0;
  }

  /* native <dialog>: global .dialog-content supplies chrome; reset UA defaults + style the backdrop */
  dialog.dialog-content {
    padding: 0;
    margin: 0;
    color: var(--text-1);
  }

  dialog.dialog-content:not([open]) {
    display: none;
  }

  dialog.dialog-content::backdrop {
    background: var(--bg-scrim);
    backdrop-filter: var(--glass-blur);
  }

  .dialog-title {
    overflow-wrap: anywhere;
  }

  .dialog-content form {
    display: flex;
    flex-direction: column;
    min-height: 0;
    flex: 1;
  }

  .form-error {
    font-size: var(--text-sm);
    color: var(--danger);
    margin: 0;
  }
</style>
