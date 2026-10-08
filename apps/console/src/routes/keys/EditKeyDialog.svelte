<script lang="ts">
  import { api } from '$lib/api.js';
  import type { ClientKey, KeyPatch } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { Button } from '$lib/components/index.js';
  import { Dialog } from 'bits-ui';
  import { XIcon } from 'lucide-svelte';
  import { toast } from 'svelte-sonner';
  import { splitList, toDateInput } from './keyLimits.js';

  interface Props {
    open: boolean;
    /** Key being edited; form fields are seeded from it each time the dialog opens. */
    editing: ClientKey | null;
    onsaved: () => void | Promise<void>;
  }

  let { open = $bindable(), editing, onsaved }: Props = $props();

  let editError = $state('');
  let editSaving = $state(false);
  let editName = $state('');
  let editExpiresOn = $state('');
  let editModels = $state('');
  let editIps = $state('');
  let editTokens = $state<number | null>(null);
  let editRpm = $state<number | null>(null);
  let editNoLog = $state(false);

  // Seed the form whenever the dialog opens for a key.
  $effect(() => {
    if (!open || !editing) return;
    editName = editing.name;
    editExpiresOn = toDateInput(editing.expires_at);
    editModels = editing.allowed_models.join(', ');
    editIps = editing.allowed_ips.join(', ');
    editTokens = editing.monthly_token_limit ?? null;
    editRpm = editing.requests_per_minute ?? null;
    editNoLog = editing.no_log;
    editError = '';
  });

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
      open = false;
      toast.success(m.key_updated());
      await onsaved();
    } catch (err) {
      // Backend 400s (bad CIDR, past date…) stay in the dialog.
      editError = (err as Error).message;
    } finally {
      editSaving = false;
    }
  }
</script>

<Dialog.Root bind:open>
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
          <span id="edit-expires-hint" class="field-hint">{m.key_expiry_hint()}</span>
        </div>
        <div class="field">
          <label for="edit-models">{m.key_allowed_models()}</label>
          <textarea id="edit-models" rows="2" bind:value={editModels} aria-describedby="edit-models-hint"></textarea>
          <span id="edit-models-hint" class="field-hint">{m.key_allowed_models_hint()}</span>
        </div>
        <div class="field">
          <label for="edit-ips">{m.key_allowed_ips()}</label>
          <textarea id="edit-ips" rows="2" bind:value={editIps} aria-describedby="edit-ips-hint"></textarea>
          <span id="edit-ips-hint" class="field-hint">{m.key_allowed_ips_hint()}</span>
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
        <span id="edit-unlimited-hint" class="field-hint">{m.key_unlimited_hint()}</span>
        <label class="check-row">
          <input type="checkbox" bind:checked={editNoLog} aria-describedby="edit-no-log-hint" />
          <span>{m.key_no_log()}</span>
        </label>
        <span id="edit-no-log-hint" class="field-hint">{m.key_no_log_hint()}</span>

        {#if editError}
          <p class="error-msg" role="alert">{editError}</p>
        {/if}

        <div class="dialog-footer">
          <Button variant="outline" onclick={() => (open = false)}>{m.common_cancel()}</Button>
          <Button type="submit" disabled={editSaving}>{m.common_save()}</Button>
        </div>
      </form>
    </Dialog.Content>
  </Dialog.Portal>
</Dialog.Root>

<style>
  .edit-form {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-5);
    overflow-y: auto;
    flex: 1 1 auto;
    min-height: 0;
  }

  /* Footer lives inside the padded form: plain button row, no extra chrome. */
  .edit-form .dialog-footer {
    padding: 0;
    border-top: 0;
  }

  .field { min-width: 0; }
  .field textarea { resize: vertical; }

  .check-row {
    flex-direction: row;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-2);
    min-height: var(--control-h);
    font-size: var(--text-base);
    font-weight: var(--weight-regular);
    color: var(--text-1);
    cursor: pointer;
  }

  .check-row input { width: var(--icon); height: var(--icon); flex-shrink: 0; }
</style>
