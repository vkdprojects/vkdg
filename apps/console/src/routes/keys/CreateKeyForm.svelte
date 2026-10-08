<script lang="ts">
  import { api } from '$lib/api.js';
  import type { CreatedKey, KeyLimits, KeyScope } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { Button } from '$lib/components/index.js';
  import { splitList } from './keyLimits.js';

  interface Props {
    /** Fired after a successful create; the page reveals the secret and reloads the list. */
    oncreated: (key: CreatedKey) => void | Promise<void>;
    /** Fired when a create starts, so the page can hide any previously revealed secret. */
    onstart: () => void;
  }

  let { oncreated, onstart }: Props = $props();

  let submitting = $state(false);
  let formError = $state('');

  let keyName = $state('');
  let scopes = $state<KeyScope[]>(['data_inference', 'data_image']);
  let expiresOn = $state('');
  let allowedModels = $state('');
  let allowedIps = $state('');
  let monthlyTokens = $state<number | null>(null);
  let rpm = $state<number | null>(null);
  let noLog = $state(false);

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

  async function createKey(e: Event) {
    e.preventDefault();
    if (!keyName.trim()) { formError = m.key_name_required(); return; }
    if (scopes.length === 0) { formError = m.key_scope_required(); return; }
    submitting = true;
    formError = '';
    onstart();
    try {
      const result = await api.createKey(keyName.trim(), scopes, limits());
      keyName = '';
      expiresOn = '';
      allowedModels = '';
      allowedIps = '';
      monthlyTokens = null;
      rpm = null;
      noLog = false;
      await oncreated(result);
    } catch (err) {
      formError = (err as Error).message;
    } finally {
      submitting = false;
    }
  }
</script>

<section aria-labelledby="create-heading" class="panel">
  <div class="panel-head">
    <h2 id="create-heading">{m.key_create()}</h2>
  </div>
  <div class="panel-body">
    <form onsubmit={createKey} class="create-form">
      <div class="create-columns">
        <div class="create-column">
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
        </div>

        <fieldset class="role-group limits create-column">
          <legend>{m.key_limits()}</legend>
          <div class="field">
            <label for="key-expires">{m.key_expires_on()}</label>
            <input id="key-expires" type="date" bind:value={expiresOn} />
          </div>
          <div class="field">
            <label for="key-models">{m.key_allowed_models()}</label>
            <textarea id="key-models" rows="2" bind:value={allowedModels} placeholder="claude-*, gpt-5*" aria-describedby="key-models-hint"></textarea>
            <span id="key-models-hint" class="field-hint">{m.key_allowed_models_hint()}</span>
          </div>
          <div class="field">
            <label for="key-ips">{m.key_allowed_ips()}</label>
            <textarea id="key-ips" rows="2" bind:value={allowedIps} placeholder="10.0.0.0/8" aria-describedby="key-ips-hint"></textarea>
            <span id="key-ips-hint" class="field-hint">{m.key_allowed_ips_hint()}</span>
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
          <span id="key-unlimited-hint" class="field-hint">{m.key_unlimited_hint()}</span>
          <label class="check-row">
            <input type="checkbox" bind:checked={noLog} aria-describedby="key-no-log-hint" />
            <span>{m.key_no_log()}</span>
          </label>
          <span id="key-no-log-hint" class="field-hint">{m.key_no_log_hint()}</span>
        </fieldset>
      </div>

      <!-- Backend 400s (bad CIDR, bad date…) render here, next to the form. -->
      {#if formError}
        <p class="error-msg" role="alert">{formError}</p>
      {/if}

      <div class="form-actions">
        <Button type="submit" disabled={submitting}>{m.key_create()}</Button>
      </div>
    </form>
  </div>
</section>

<style>
  .create-form {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }

  .create-columns {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, var(--col-lg)), 1fr));
    gap: var(--space-5);
    align-items: start;
  }

  .create-column {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-width: 0;
  }

  .field { min-width: 0; }
  .field textarea { resize: vertical; }

  .role-group {
    border: var(--border-w) solid var(--border);
    border-radius: var(--radius);
    background: var(--bg-inset);
    padding: 0 var(--space-4) var(--space-4);
    margin: 0;
    min-width: 0;
  }

  .role-group legend {
    font-size: var(--text-sm);
    font-weight: var(--weight-semibold);
    color: var(--text-1);
    padding: 0 var(--space-2);
  }

  .limits {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding-top: var(--space-2);
  }

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

  .role-option {
    flex-direction: row;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-1);
    min-height: var(--control-h-lg);
    cursor: pointer;
    border-radius: var(--radius-sm);
    font-weight: var(--weight-regular);
  }

  .role-option:not(:last-child) {
    border-bottom: var(--border-w) solid var(--border);
  }

  .role-option input[type='checkbox'] {
    margin-top: var(--space-0);
    width: var(--icon);
    height: var(--icon);
    flex-shrink: 0;
  }

  .role-info {
    display: flex;
    flex-direction: column;
    gap: var(--space-0);
    min-width: 0;
  }

  .role-label {
    font-size: var(--text-base);
    font-weight: var(--weight-medium);
    color: var(--text-1);
  }

  .role-desc {
    font-size: var(--text-xs);
    font-weight: var(--weight-regular);
    color: var(--text-3);
  }

  .form-actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
  }
</style>
