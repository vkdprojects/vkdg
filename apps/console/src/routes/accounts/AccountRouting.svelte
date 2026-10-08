<script lang="ts">
  import { toast } from 'svelte-sonner';
  import { api } from '$lib/api.js';
  import type { Account, ConnectionPatch, ConnectionSummary, ConnectionTestResult } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { formatDateTime } from '$lib/format.js';
  import { connectionStatusLabel } from '$lib/status.js';
  import { Badge, Button, Input, Spinner, StatusDot } from '$lib/components/index.js';

  interface Props {
    account: Account;
    /** Already filtered to this account. Normally one; several are all shown. */
    connections: ConnectionSummary[];
    /** Called after a change landed on the server; awaited so edits clear once fresh data is in. */
    onchanged: () => void | Promise<void>;
  }

  let { account, connections, onchanged }: Props = $props();

  /** Raw field text the user typed; an absent field shows the server value. */
  interface Edit {
    models?: string;
    weight?: string;
    maxConcurrent?: string;
  }

  let edits = $state<Record<string, Edit>>({});
  let saving = $state<string | null>(null);
  let testing = $state<string | null>(null);
  let testResults = $state<Record<string, ConnectionTestResult>>({});
  let enabling = $state(false);

  function parseModels(text: string): string[] {
    return text.split(',').map((s) => s.trim()).filter(Boolean);
  }

  function parseCount(text: string): number | null {
    const t = text.trim();
    return /^\d+$/.test(t) && Number(t) >= 1 ? Number(t) : null;
  }

  /** A number input hands back a number at runtime and `null` when empty. */
  function fieldText(value: unknown): string {
    return value == null ? '' : String(value);
  }

  function setEdit(id: string, field: keyof Edit, value: unknown) {
    edits[id] = { ...edits[id], [field]: fieldText(value) };
  }

  /** What would be sent, plus per-field validation errors; only edited fields are checked. */
  function draft(c: ConnectionSummary): { patch: ConnectionPatch; errors: Record<keyof Edit, string | undefined> } {
    const e = edits[c.id] ?? {};
    const patch: ConnectionPatch = {};
    const errors: Record<keyof Edit, string | undefined> = { models: undefined, weight: undefined, maxConcurrent: undefined };

    if (e.models !== undefined) {
      const list = parseModels(e.models);
      if (list.length === 0) errors.models = m.routing_models_required();
      else if (list.join('\n') !== c.models.join('\n')) patch.models = list;
    }
    if (e.weight !== undefined) {
      const n = parseCount(e.weight);
      if (n === null) errors.weight = m.routing_count_invalid();
      else if (n !== c.weight) patch.weight = n;
    }
    if (e.maxConcurrent !== undefined) {
      const n = parseCount(e.maxConcurrent);
      if (n === null) errors.maxConcurrent = m.routing_count_invalid();
      else if (n !== c.max_concurrent) patch.max_concurrent = n;
    }
    return { patch, errors };
  }

  async function save(c: ConnectionSummary) {
    const { patch, errors } = draft(c);
    if (errors.models || errors.weight || errors.maxConcurrent || Object.keys(patch).length === 0) return;
    saving = c.id;
    try {
      await api.patchConnection(c.id, patch);
      toast.success(m.routing_saved());
      await onchanged();
      delete edits[c.id];
    } catch (e) {
      toast.error((e as Error).message);
    } finally {
      saving = null;
    }
  }

  async function test(id: string) {
    testing = id;
    try {
      testResults[id] = await api.testConnection(id);
    } catch (e) {
      testResults[id] = { latency_ms: 0, ok: false, error: (e as Error).message };
    } finally {
      testing = null;
    }
  }

  async function enable() {
    enabling = true;
    try {
      await api.enableAccount(account.id);
      await onchanged();
    } catch (e) {
      toast.error((e as Error).message);
    } finally {
      enabling = false;
    }
  }
</script>

<section class="routing" aria-label={m.routing_region({ label: account.label })}>
  <p class="eyebrow">{m.routing_title()}</p>

  {#if connections.length === 0}
    <div class="idle">
      <span class="idle-note">{m.routing_idle()}</span>
      <Button variant="outline" size="sm" disabled={enabling} onclick={enable} ariaLabel={`${m.routing_enable()} ${account.label}`}>
        {#if enabling}<Spinner size="sm" />{:else}{m.routing_enable()}{/if}
      </Button>
    </div>
  {:else}
    {#each connections as c (c.id)}
      {@const d = draft(c)}
      {@const edit = edits[c.id]}
      {@const result = testResults[c.id]}
      <form
        class="connection"
        onsubmit={(event) => { event.preventDefault(); save(c); }}
      >
        <div class="conn-head">
          <div class="conn-status">
            <StatusDot status={c.status} />
            <Badge status={c.status} label={connectionStatusLabel(c.status)} />
            {#if connections.length > 1}<span class="conn-id mono">{c.id}</span>{/if}
          </div>
          {#if c.cooldown_until || c.failure_count != null}
            <div class="conn-meta">
              {#if c.cooldown_until}<span>{m.connection_cooldown_until({ time: formatDateTime(c.cooldown_until) })}</span>{/if}
              {#if c.failure_count != null}<span>{m.connection_failures({ n: c.failure_count })}</span>{/if}
            </div>
          {/if}
        </div>

        <Input
          id={`routing-${c.id}-models`}
          label={m.connection_models()}
          bind:value={() => edit?.models ?? c.models.join(', '), (v) => setEdit(c.id, 'models', v)}
          hint={m.routing_models_hint()}
          error={d.errors.models}
          disabled={saving === c.id}
          autocomplete="off"
        />

        <div class="numbers">
          <Input
            id={`routing-${c.id}-weight`}
            type="number"
            min={1}
            label={m.routing_weight()}
            bind:value={() => edit?.weight ?? String(c.weight), (v) => setEdit(c.id, 'weight', v)}
            error={d.errors.weight}
            disabled={saving === c.id}
          />
          <Input
            id={`routing-${c.id}-max`}
            type="number"
            min={1}
            label={m.routing_max_concurrent()}
            bind:value={() => edit?.maxConcurrent ?? String(c.max_concurrent), (v) => setEdit(c.id, 'maxConcurrent', v)}
            error={d.errors.maxConcurrent}
            disabled={saving === c.id}
          />
        </div>

        <div class="conn-actions">
          <span class="test-result" role="status">
            {#if result}
              <span class="test-badge" class:ok={result.ok} class:err={!result.ok}>
                {result.ok ? m.routing_test_ok({ ms: result.latency_ms }) : m.routing_test_failed({ error: result.error ?? '' })}
              </span>
            {/if}
          </span>
          <Button variant="outline" size="sm" disabled={testing === c.id} onclick={() => test(c.id)}>
            {#if testing === c.id}<Spinner size="sm" />{:else}{m.connection_test()}{/if}
          </Button>
          <Button
            type="submit"
            size="sm"
            disabled={saving === c.id || Object.keys(d.patch).length === 0 || !!(d.errors.models || d.errors.weight || d.errors.maxConcurrent)}
          >
            {#if saving === c.id}<Spinner size="sm" />{:else}{m.common_save()}{/if}
          </Button>
        </div>
      </form>
    {/each}
  {/if}
</section>

<style>
  .routing {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
  }

  .eyebrow {
    margin: 0;
    color: var(--text-1);
    font-size: var(--text-sm);
    font-weight: var(--weight-semibold);
  }

  .idle {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    flex-wrap: wrap;
    padding: var(--space-3) var(--space-4);
    background: var(--bg-inset);
    border: var(--border-w) dashed var(--border-strong);
    border-radius: var(--radius);
  }

  .idle-note {
    color: var(--text-2);
    font-size: var(--text-sm);
    min-width: 0;
    flex: 1 1 var(--col-sm);
  }

  .connection {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
    background: var(--bg-inset);
    border: var(--border-w) solid var(--border);
    border-radius: var(--radius);
    min-width: 0;
  }

  .conn-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    flex-wrap: wrap;
  }

  .conn-status {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
    min-width: 0;
  }

  .conn-id {
    color: var(--text-3);
    font-size: var(--text-2xs);
    overflow-wrap: anywhere;
  }

  .conn-meta {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1) var(--space-3);
    color: var(--text-3);
    font-size: var(--text-xs);
  }

  .numbers {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, var(--col-xs)), 1fr));
    gap: var(--space-3);
  }

  .conn-actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .test-result {
    margin-right: auto;
    min-width: 0;
  }

  .test-badge {
    display: inline-block;
    border-radius: var(--radius-full);
    font-size: var(--text-xs);
    font-weight: var(--weight-medium);
    padding: var(--space-0) var(--space-2);
    overflow-wrap: anywhere;
  }

  .test-badge.ok {
    background: var(--success-subtle);
    color: var(--success);
  }

  .test-badge.err {
    background: var(--danger-subtle);
    color: var(--danger);
  }
</style>
