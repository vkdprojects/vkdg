<script lang="ts">
  import type { PageData, ActionData } from './$types';
  import { m } from '$lib/paraglide/messages.js';
  import { CopyButton, EmptyState, Button } from '$lib/components/index.js';

  let { data, form }: { data: PageData; form: ActionData } = $props();

  function formatDate(iso: string): string {
    return new Intl.DateTimeFormat(undefined, {
      year: 'numeric',
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    }).format(new Date(iso));
  }
</script>

<div class="page">
  <div class="page-header">
    <h1 class="page-title">{m.nav_keys()}</h1>
  </div>

  <section aria-labelledby="create-heading" class="create-section">
    <h2 id="create-heading">{m.key_create()}</h2>
    <form method="POST" action="?/create" class="create-form">
      <div class="field">
        <label for="key-name">{m.key_name()}</label>
        <input id="key-name" type="text" name="name" required placeholder="e.g. ci-runner" />
      </div>

      <fieldset class="role-group">
        <legend>{m.key_role()}</legend>
        <label class="role-option">
          <input type="radio" name="role" value="viewer" checked />
          <span class="role-info">
            <span class="role-label">{m.key_role_viewer()}</span>
            <span class="role-desc">{m.key_role_viewer_desc()}</span>
          </span>
        </label>
        <label class="role-option">
          <input type="radio" name="role" value="operator" />
          <span class="role-info">
            <span class="role-label">{m.key_role_operator()}</span>
            <span class="role-desc">{m.key_role_operator_desc()}</span>
          </span>
        </label>
        <label class="role-option">
          <input type="radio" name="role" value="admin" />
          <span class="role-info">
            <span class="role-label">{m.key_role_admin()}</span>
            <span class="role-desc">{m.key_role_admin_desc()}</span>
          </span>
        </label>
      </fieldset>

      <Button type="submit">{m.key_create()}</Button>
    </form>

    {#if form?.error}
      <p class="error-msg" role="alert">{form.error}</p>
    {/if}

    {#if form?.created}
      <div class="created-key" role="alert">
        <div class="created-header">
          <svg class="warning-icon" xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z"/>
            <path d="M12 9v4"/><path d="M12 17h.01"/>
          </svg>
          <strong class="created-notice">{m.key_store_warning()}</strong>
        </div>
        <div class="key-box">
          <code class="key-value">{form.created.key}</code>
        </div>
        <div class="key-copy-row">
          <CopyButton text={form.created.key} />
        </div>
      </div>
    {/if}
  </section>

  <section aria-labelledby="keys-heading">
    <h2 id="keys-heading">{m.nav_keys()} ({data.keys.length})</h2>
    {#if data.keys.length === 0}
      <EmptyState title={m.key_empty()} description="Create a key to authenticate API clients." />
    {:else}
      <table>
        <thead>
          <tr>
            <th scope="col">{m.key_name()}</th>
            <th scope="col">{m.key_role()}</th>
            <th scope="col">{m.key_created()}</th>
            <th scope="col">{m.key_last_used()}</th>
            <th scope="col"></th>
          </tr>
        </thead>
        <tbody>
          {#each data.keys as k (k.id)}
            <tr>
              <td class="key-name-cell">{k.name}</td>
              <td><span class="role-badge role-{k.role}">{k.role}</span></td>
              <td class="date-cell">{formatDate(k.created_at)}</td>
              <td class="date-cell">{k.last_used_at ? formatDate(k.last_used_at) : m.key_never()}</td>
              <td class="action-cell">
                <form method="POST" action="?/revoke">
                  <input type="hidden" name="id" value={k.id} />
                  <Button variant="danger" size="sm" type="submit">{m.key_revoke()}</Button>
                </form>
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

<style>
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

  /* Role radio group */
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

  .role-option input[type='radio'] {
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

  /* One-time key display */
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

  /* Error */
  .error-msg {
    margin-top: 8px;
    font-size: 0.8125rem;
    color: var(--danger);
  }

  /* Table */
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

  /* Role badge */
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

  /* Instructions */
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
