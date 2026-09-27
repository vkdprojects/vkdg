<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api.js';
  import type { ClientKey, CreatedKey, KeyLimits, KeyPatch, KeyScope } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { CopyButton, EmptyState, Button, Spinner } from '$lib/components/index.js';
  import { Dialog } from 'bits-ui';
  import { XIcon } from 'lucide-svelte';
  import { toast } from 'svelte-sonner';

  let keys = $state<ClientKey[]>([]);
  let loading = $state(true);
  let submitting = $state(false);
  let formError = $state('');
  let createdKey = $state<CreatedKey | null>(null);

  let keyName = $state('');
  let scopes = $state<KeyScope[]>(['data_inference', 'data_image']);
  let expiresOn = $state('');
  let allowedModels = $state('');
  let allowedIps = $state('');
  let monthlyTokens = $state<number | null>(null);
  let rpm = $state<number | null>(null);
  let noLog = $state(false);

  const splitList = (v: string) => v.split(/[\n,]/).map((x) => x.trim()).filter(Boolean);

  /** Only filled fields are sent; the backend treats absent as unrestricted. */
  function limits(): KeyLimits {
    const out: KeyLimits = {};
    // Date input is local; the key stops working at the end of that day.
    if (expiresOn) out.expires_at = new Date(`${expiresOn}T23:59:59`).toISOString();
    const models = splitList(allowedModels);
    if (models.length) out.allowed_models = models;
    const ips = splitList(allowedIps);
    if (ips.length) out.allowed_ips = ips;
    if (monthlyTokens != null) out.monthly_token_limit = monthlyTokens;
    if (rpm != null) out.requests_per_minute = rpm;
    if (noLog) out.no_log = true;
    return out;
  }

  const statusLabels: Record<ClientKey['status'], () => string> = {
    active: m.key_status_active,
    disabled: m.key_status_disabled,
    expired: m.key_status_expired,
    revoked: m.key_status_revoked,
  };

  // ── Row actions ──────────────────────────────────────────────────────────

  /** Raw secret from a regenerate; shown once, next to the table. */
  let regenerated = $state<CreatedKey | null>(null);
  let busyId = $state<string | null>(null);

  let editing = $state<ClientKey | null>(null);
  let editOpen = $state(false);
  let editError = $state('');
  let editSaving = $state(false);
  let editName = $state('');
  let editExpiresOn = $state('');
  let editModels = $state('');
  let editIps = $state('');
  let editTokens = $state<number | null>(null);
  let editRpm = $state<number | null>(null);
  let editNoLog = $state(false);

  /** Local `YYYY-MM-DD` for a date input, matching how create sets end-of-day. */
  function toDateInput(iso: string | null | undefined): string {
    if (!iso) return '';
    const d = new Date(iso);
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
  }

  function openEdit(k: ClientKey) {
    editing = k;
    editName = k.name;
    editExpiresOn = toDateInput(k.expires_at);
    editModels = k.allowed_models.join(', ');
    editIps = k.allowed_ips.join(', ');
    editTokens = k.monthly_token_limit ?? null;
    editRpm = k.requests_per_minute ?? null;
    editNoLog = k.no_log;
    editError = '';
    editOpen = true;
  }

  /** Only changed fields; a cleared expiry or limit becomes `null`. */
  function buildPatch(k: ClientKey): KeyPatch {
    const p: KeyPatch = {};
    const name = editName.trim();
    if (name !== k.name) p.name = name;
    if (editExpiresOn !== toDateInput(k.expires_at)) {
      p.expires_at = editExpiresOn ? new Date(`${editExpiresOn}T23:59:59`).toISOString() : null;
    }
    const models = splitList(editModels);
    if (models.join('\n') !== k.allowed_models.join('\n')) p.allowed_models = models;
    const ips = splitList(editIps);
    if (ips.join('\n') !== k.allowed_ips.join('\n')) p.allowed_ips = ips;
    // An emptied number input binds to null (or undefined); both mean "clear".
    const tokens = editTokens ?? null;
    if (tokens !== (k.monthly_token_limit ?? null)) p.monthly_token_limit = tokens;
    const rpmVal = editRpm ?? null;
    if (rpmVal !== (k.requests_per_minute ?? null)) p.requests_per_minute = rpmVal;
    if (editNoLog !== k.no_log) p.no_log = editNoLog;
    return p;
  }

  async function saveEdit(e: Event) {
    e.preventDefault();
    if (!editing) return;
    if (!editName.trim()) { editError = m.key_name_required(); return; }
    const patch = buildPatch(editing);
    if (Object.keys(patch).length === 0) { editError = m.key_no_changes(); return; }
    editSaving = true;
    editError = '';
    try {
      await api.updateKey(editing.id, patch);
      editOpen = false;
      toast.success(m.key_updated());
      keys = (await api.listKeys()).items;
    } catch (err) {
      // Backend 400s (bad CIDR, past date…) stay in the dialog.
      editError = (err as Error).message;
    } finally {
      editSaving = false;
    }
  }

  async function regenerateKey(k: ClientKey) {
    if (!confirm(m.key_regenerate_confirm())) return;
    busyId = k.id;
    try {
      regenerated = await api.regenerateKey(k.id);
      keys = (await api.listKeys()).items;
    } catch (e) {
      toast.error((e as Error).message);
    } finally {
      busyId = null;
    }
  }

  async function toggleDisabled(k: ClientKey) {
    const disable = k.status !== 'disabled';
    if (disable && !confirm(m.key_disable_confirm())) return;
    busyId = k.id;
    try {
      await (disable ? api.disableKey(k.id) : api.enableKey(k.id));
      toast.success(disable ? m.key_disabled_toast() : m.key_enabled_toast());
      keys = (await api.listKeys()).items;
    } catch (e) {
      toast.error((e as Error).message);
    } finally {
      busyId = null;
    }
  }

  function formatDate(iso: string): string {
    return new Intl.DateTimeFormat(undefined, {
      year: 'numeric',
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    }).format(new Date(iso));
  }

  onMount(async () => {
    try {
      const res = await api.listKeys();
      keys = res.items;
    } catch (e) {
      toast.error((e as Error).message);
    } finally {
      loading = false;
    }
  });

  async function createKey(e: Event) {
    e.preventDefault();
    if (!keyName.trim()) { formError = m.key_name_required(); return; }
    if (scopes.length === 0) { formError = m.key_scope_required(); return; }
    submitting = true;
    formError = '';
    createdKey = null;
    try {
      const result = await api.createKey(keyName.trim(), scopes, limits());
      createdKey = result;
      keyName = '';
      expiresOn = '';
      allowedModels = '';
      allowedIps = '';
      monthlyTokens = null;
      rpm = null;
      noLog = false;
      const res = await api.listKeys();
      keys = res.items;
    } catch (err) {
      formError = (err as Error).message;
    } finally {
      submitting = false;
    }
  }

  async function revokeKey(id: string) {
    if (!confirm(m.key_revoke_confirm())) return;
    try {
      await api.revokeKey(id);
      // Revoked keys stay listed so the operator can see what was cut off.
      keys = (await api.listKeys()).items;
    } catch (e) {
      toast.error((e as Error).message);
    }
  }
</script>

<div class="page">
  <div class="page-header">
    <h1 class="page-title">{m.nav_keys()}</h1>
  </div>

  <section aria-labelledby="create-heading" class="create-section">
    <h2 id="create-heading">{m.key_create()}</h2>
    <form onsubmit={createKey} class="create-form">
      <div class="field">
        <label for="key-name">{m.key_name()}</label>
        <input id="key-name" type="text" bind:value={keyName} required placeholder="e.g. ci-runner" />
      </div>

      <fieldset class="role-group">
        <legend>{m.key_scopes()}</legend>
        <label class="role-option">
          <input type="checkbox" value="data_inference" bind:group={scopes} />
          <span class="role-info">
            <span class="role-label">{m.key_scope_inference()}</span>
            <span class="role-desc">{m.key_scope_inference_desc()}</span>
          </span>
        </label>
        <label class="role-option">
          <input type="checkbox" value="data_image" bind:group={scopes} />
          <span class="role-info">
            <span class="role-label">{m.key_scope_image()}</span>
            <span class="role-desc">{m.key_scope_image_desc()}</span>
          </span>
        </label>
      </fieldset>

      <fieldset class="role-group limits">
        <legend>{m.key_limits()}</legend>
        <div class="field">
          <label for="key-expires">{m.key_expires_on()}</label>
          <input id="key-expires" type="date" bind:value={expiresOn} />
        </div>
        <div class="field">
          <label for="key-models">{m.key_allowed_models()}</label>
          <textarea id="key-models" rows="2" bind:value={allowedModels} placeholder="claude-*, gpt-5*" aria-describedby="key-models-hint"></textarea>
          <span id="key-models-hint" class="hint">{m.key_allowed_models_hint()}</span>
        </div>
        <div class="field">
          <label for="key-ips">{m.key_allowed_ips()}</label>
          <textarea id="key-ips" rows="2" bind:value={allowedIps} placeholder="10.0.0.0/8" aria-describedby="key-ips-hint"></textarea>
          <span id="key-ips-hint" class="hint">{m.key_allowed_ips_hint()}</span>
        </div>
        <div class="field-row">
          <div class="field">
            <label for="key-tokens">{m.key_monthly_tokens()}</label>
            <input id="key-tokens" type="number" min="1" step="1" bind:value={monthlyTokens} aria-describedby="key-unlimited-hint" />
          </div>
          <div class="field">
            <label for="key-rpm">{m.key_rpm()}</label>
            <input id="key-rpm" type="number" min="1" step="1" bind:value={rpm} aria-describedby="key-unlimited-hint" />
          </div>
        </div>
        <span id="key-unlimited-hint" class="hint">{m.key_unlimited_hint()}</span>
        <label class="check-row">
          <input type="checkbox" bind:checked={noLog} aria-describedby="key-no-log-hint" />
          <span>{m.key_no_log()}</span>
        </label>
        <span id="key-no-log-hint" class="hint">{m.key_no_log_hint()}</span>
      </fieldset>

      <!-- Backend 400s (bad CIDR, bad date…) render here, next to the form. -->
      {#if formError}
        <p class="error-msg" role="alert">{formError}</p>
      {/if}

      <Button type="submit" disabled={submitting}>{m.key_create()}</Button>
    </form>

    {#if createdKey}
      {@render secretReveal(createdKey.key, m.key_store_warning())}
    {/if}
  </section>

  <section aria-labelledby="keys-heading">
    <h2 id="keys-heading">{m.nav_keys()} ({keys.length})</h2>
    {#if regenerated}
      {@render secretReveal(regenerated.key, `${m.key_regenerated_notice({ name: regenerated.name })}. ${m.key_store_warning()}`)}
    {/if}
    {#if loading}
      <div class="loading"><Spinner size="sm" /> {m.common_loading()}</div>
    {:else if keys.length === 0}
      <EmptyState title={m.key_empty()} description={m.key_empty_desc()} />
    {:else}
      <table>
        <thead>
          <tr>
            <th scope="col">{m.key_name()}</th>
            <th scope="col">{m.key_prefix()}</th>
            <th scope="col">{m.key_scopes()}</th>
            <th scope="col">{m.key_status()}</th>
            <th scope="col">{m.key_expires()}</th>
            <th scope="col">{m.key_restrictions()}</th>
            <th scope="col">{m.key_usage_month()}</th>
            <th scope="col">{m.key_created()}</th>
            <th scope="col">{m.key_last_used()}</th>
            <th scope="col"></th>
          </tr>
        </thead>
        <tbody>
          {#each keys as k (k.id)}
            <tr>
              <td class="key-name-cell">{k.name}</td>
              <td><code>{k.prefix}…</code></td>
              <td>
                {#each k.scopes as sc}
                  <span class="role-badge role-viewer">{sc === 'data_image' ? m.key_scope_image() : m.key_scope_inference()}</span>
                {/each}
              </td>
              <td>
                <!-- Text, not color alone, carries the state. -->
                <span class="role-badge {k.status === 'active' ? 'role-operator' : k.status === 'disabled' ? 'role-viewer' : 'role-admin'}">
                  {statusLabels[k.status]()}
                </span>
              </td>
              <td class="date-cell">{k.expires_at ? formatDate(k.expires_at) : m.key_never()}</td>
              <td class="restrictions-cell">
                {#if k.allowed_models.length === 0 && k.allowed_ips.length === 0 && !k.monthly_token_limit && !k.requests_per_minute && !k.no_log}
                  {m.key_no_restrictions()}
                {:else}
                  {#if k.allowed_models.length}<div>{m.key_models_count({ list: k.allowed_models.join(', ') })}</div>{/if}
                  {#if k.allowed_ips.length}<div>{m.key_ips_count({ list: k.allowed_ips.join(', ') })}</div>{/if}
                  {#if k.requests_per_minute}<div>{m.key_rpm_summary({ rpm: k.requests_per_minute })}</div>{/if}
                  {#if k.no_log}<div>{m.key_no_log_summary()}</div>{/if}
                {/if}
              </td>
              <td class="usage-cell">
                {#if k.usage_this_month}
                  {@const used = k.usage_this_month.input_tokens + k.usage_this_month.output_tokens}
                  <div>{m.key_usage_tokens({ used: used.toLocaleString(), limit: k.monthly_token_limit ? k.monthly_token_limit.toLocaleString() : '∞' })}</div>
                  <div class="hint">{m.key_usage_requests({ n: k.usage_this_month.requests })}</div>
                  {#if k.monthly_token_limit}
                    <progress max={k.monthly_token_limit} value={Math.min(used, k.monthly_token_limit)} aria-label={m.key_usage_month()}></progress>
                  {/if}
                {:else}
                  —
                {/if}
              </td>
              <td class="date-cell">{formatDate(k.created_at)}</td>
              <td class="date-cell">{k.last_used_at ? formatDate(k.last_used_at) : m.key_never()}</td>
              <td class="action-cell">
                {#if k.status !== 'revoked'}
                  <div class="row-actions">
                    <Button variant="outline" size="sm" onclick={() => openEdit(k)} ariaLabel={`${m.key_edit()} ${k.name}`}>{m.key_edit()}</Button>
                    <Button variant="outline" size="sm" disabled={busyId === k.id} onclick={() => regenerateKey(k)} ariaLabel={`${m.key_regenerate()} ${k.name}`}>{m.key_regenerate()}</Button>
                    {#if k.status === 'disabled'}
                      <Button variant="outline" size="sm" disabled={busyId === k.id} onclick={() => toggleDisabled(k)} ariaLabel={`${m.key_enable()} ${k.name}`}>{m.key_enable()}</Button>
                    {:else}
                      <Button variant="ghost" size="sm" disabled={busyId === k.id} onclick={() => toggleDisabled(k)} ariaLabel={`${m.key_disable()} ${k.name}`}>{m.key_disable()}</Button>
                    {/if}
                    <Button variant="danger" size="sm" onclick={() => revokeKey(k.id)} ariaLabel={`${m.key_revoke()} ${k.name}`}>{m.key_revoke()}</Button>
                  </div>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>

  <section aria-labelledby="claude-heading">
    <h2 id="claude-heading">{m.key_claude_code_heading()}</h2>
    <p class="instructions">
      Set <code>ANTHROPIC_BASE_URL=http://localhost:8080</code> and
      <code>ANTHROPIC_API_KEY=&lt;your-key&gt;</code> in your Claude Code config.
    </p>
  </section>
</div>

{#snippet secretReveal(secret: string, notice: string)}
  <div class="created-key" role="alert">
    <div class="created-header">
      <svg class="warning-icon" xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z"/>
        <path d="M12 9v4"/><path d="M12 17h.01"/>
      </svg>
      <strong class="created-notice">{notice}</strong>
    </div>
    <div class="key-box">
      <code class="key-value">{secret}</code>
    </div>
    <div class="key-copy-row">
      <CopyButton text={secret} />
    </div>
  </div>
{/snippet}

<Dialog.Root bind:open={editOpen}>
  <Dialog.Portal>
    <Dialog.Overlay class="dialog-overlay" />
    <Dialog.Content class="dialog-content" aria-describedby={undefined}>
      <div class="dialog-header">
        <Dialog.Title class="dialog-title">{m.key_edit_title({ name: editing?.name ?? '' })}</Dialog.Title>
        <Dialog.Close class="dialog-close" aria-label={m.common_close()}>
          <XIcon size={16} aria-hidden="true" />
        </Dialog.Close>
      </div>
      <form onsubmit={saveEdit} class="edit-form">
        <div class="field">
          <label for="edit-name">{m.key_name()}</label>
          <input id="edit-name" type="text" bind:value={editName} required />
        </div>
        <div class="field">
          <label for="edit-expires">{m.key_expires_on()}</label>
          <input id="edit-expires" type="date" bind:value={editExpiresOn} aria-describedby="edit-expires-hint" />
          <span id="edit-expires-hint" class="hint">{m.key_expiry_hint()}</span>
        </div>
        <div class="field">
          <label for="edit-models">{m.key_allowed_models()}</label>
          <textarea id="edit-models" rows="2" bind:value={editModels} aria-describedby="edit-models-hint"></textarea>
          <span id="edit-models-hint" class="hint">{m.key_allowed_models_hint()}</span>
        </div>
        <div class="field">
          <label for="edit-ips">{m.key_allowed_ips()}</label>
          <textarea id="edit-ips" rows="2" bind:value={editIps} aria-describedby="edit-ips-hint"></textarea>
          <span id="edit-ips-hint" class="hint">{m.key_allowed_ips_hint()}</span>
        </div>
        <div class="field-row">
          <div class="field">
            <label for="edit-tokens">{m.key_monthly_tokens()}</label>
            <input id="edit-tokens" type="number" min="1" step="1" bind:value={editTokens} aria-describedby="edit-unlimited-hint" />
          </div>
          <div class="field">
            <label for="edit-rpm">{m.key_rpm()}</label>
            <input id="edit-rpm" type="number" min="1" step="1" bind:value={editRpm} aria-describedby="edit-unlimited-hint" />
          </div>
        </div>
        <span id="edit-unlimited-hint" class="hint">{m.key_unlimited_hint()}</span>
        <label class="check-row">
          <input type="checkbox" bind:checked={editNoLog} aria-describedby="edit-no-log-hint" />
          <span>{m.key_no_log()}</span>
        </label>
        <span id="edit-no-log-hint" class="hint">{m.key_no_log_hint()}</span>

        {#if editError}
          <p class="error-msg" role="alert">{editError}</p>
        {/if}

        <div class="dialog-footer">
          <Button variant="outline" onclick={() => (editOpen = false)}>{m.common_cancel()}</Button>
          <Button type="submit" disabled={editSaving}>{m.common_save()}</Button>
        </div>
      </form>
    </Dialog.Content>
  </Dialog.Portal>
</Dialog.Root>

<style>
  .loading {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-3);
    font-size: 0.875rem;
    padding: 16px 0;
  }

  .create-section {
    margin-bottom: 36px;
  }

  .create-form {
    display: flex;
    flex-direction: column;
    gap: 16px;
    max-width: 480px;
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

  .field input,
  .field textarea {
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

  .field textarea {
    font-family: inherit;
    resize: vertical;
  }

  .field input:focus,
  .field textarea:focus {
    border-color: var(--accent);
    outline: none;
  }

  .field input::placeholder {
    color: var(--text-3);
  }

  .role-group {
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 0 12px 12px;
    margin: 0;
  }

  .role-group legend {
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--text-2);
    padding: 0 4px;
  }

  .limits {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding-top: 8px;
  }

  .field-row {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }

  .hint {
    font-size: 0.75rem;
    color: var(--text-3);
  }

  .check-row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 8px;
    font-size: 0.875rem;
  }

  .restrictions-cell {
    font-size: 0.75rem;
    color: var(--text-2);
    max-width: 220px;
    overflow-wrap: anywhere;
  }

  .role-option {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 8px 4px;
    cursor: pointer;
    border-radius: var(--radius-sm);
  }

  .role-option:not(:last-child) {
    border-bottom: 1px solid var(--border);
  }

  .role-option input[type='checkbox'] {
    margin-top: 2px;
    accent-color: var(--accent);
    flex-shrink: 0;
  }

  .role-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .role-label {
    font-size: 0.875rem;
    font-weight: 500;
    color: var(--text-1);
  }

  .role-desc {
    font-size: 0.75rem;
    color: var(--text-3);
  }

  .created-key {
    margin-top: 16px;
    background: color-mix(in oklch, var(--warning) 8%, transparent);
    border: 1px solid color-mix(in oklch, var(--warning) 30%, transparent);
    border-radius: var(--radius);
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    max-width: 600px;
  }

  .created-header {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .warning-icon {
    color: var(--warning);
    flex-shrink: 0;
  }

  .created-notice {
    font-size: 0.875rem;
    color: var(--warning);
    font-weight: 600;
  }

  .key-box {
    background: var(--bg-base);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 12px 16px;
    overflow-x: auto;
  }

  .key-value {
    font-family: ui-monospace, 'Cascadia Code', 'Source Code Pro', Menlo, Consolas, monospace;
    font-size: 0.875rem;
    color: var(--text-1);
    word-break: break-all;
    white-space: pre-wrap;
  }

  .key-copy-row {
    display: flex;
  }

  .error-msg {
    margin-top: 8px;
    font-size: 0.8125rem;
    color: var(--danger);
  }

  .key-name-cell {
    font-weight: 500;
  }

  .date-cell {
    font-size: 0.8125rem;
    color: var(--text-2);
    white-space: nowrap;
  }

  .action-cell {
    text-align: right;
  }

  .row-actions {
    display: inline-flex;
    gap: 6px;
    flex-wrap: wrap;
    justify-content: flex-end;
  }

  .edit-form {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 20px;
    max-height: calc(100vh - 120px);
    overflow-y: auto;
  }

  .dialog-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 20px 20px 0;
  }

  .dialog-footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  /* Same global dialog chrome as the connections and accounts pages. */
  :global(.dialog-overlay) {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.45);
    z-index: 50;
  }

  :global(.dialog-content) {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    z-index: 51;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    width: min(480px, calc(100vw - 32px));
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.25);
  }

  :global(.dialog-title) {
    font-size: 1rem;
    font-weight: 600;
    color: var(--text-1);
    margin: 0;
  }

  :global(.dialog-close) {
    display: flex;
    background: transparent;
    border: none;
    color: var(--text-3);
    cursor: pointer;
    padding: 4px;
    border-radius: var(--radius-sm);
  }

  :global(.dialog-close:hover) {
    color: var(--text-1);
    background: var(--bg-hover);
  }

  .role-badge {
    display: inline-flex;
    align-items: center;
    padding: 2px 8px;
    border-radius: 9999px;
    font-size: 0.75rem;
    font-weight: 500;
  }

  .role-viewer {
    background: color-mix(in oklch, var(--text-3) 15%, transparent);
    color: var(--text-2);
  }

  .role-operator {
    background: color-mix(in oklch, var(--accent) 15%, transparent);
    color: var(--accent);
  }

  .role-admin {
    background: color-mix(in oklch, var(--warning) 15%, transparent);
    color: var(--warning);
  }

  .instructions {
    font-size: 0.875rem;
    color: var(--text-2);
    line-height: 1.6;
  }

  .instructions code {
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 1px 5px;
    font-size: 0.8125rem;
    color: var(--text-1);
  }
</style>
