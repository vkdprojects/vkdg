<script lang="ts">
  import { onMount } from 'svelte';
  import { AlertDialog } from 'bits-ui';
  import { toast } from 'svelte-sonner';
  import { api } from '$lib/api.js';
  import type { Account } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { Button, EmptyState, Spinner } from '$lib/components/index.js';
  import ConnectAccountModal from './ConnectAccountModal.svelte';

  let accounts = $state<Account[]>([]);
  let loading = $state(true);
  let connectOpen = $state(false);
  let connectProvider = $state('kiro');
  let pendingDelete = $state<Account | null>(null);
  let deleting = $state(false);

  function formatDate(iso: string): string {
    return new Intl.DateTimeFormat(undefined, {
      year: 'numeric',
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    }).format(new Date(iso));
  }

  async function refresh() {
    try {
      accounts = (await api.listAccounts()).items;
    } catch (e) {
      toast.error((e as Error).message);
    } finally {
      loading = false;
    }
  }

  onMount(refresh);

  function connect(provider = 'kiro') {
    connectProvider = provider;
    connectOpen = true;
  }

  function onConnected(account: Account) {
    toast.success(m.acct_connected({ label: account.label }));
    refresh();
  }

  async function confirmDelete() {
    if (!pendingDelete) return;
    deleting = true;
    try {
      await api.deleteAccount(pendingDelete.id);
      toast.success(m.acct_deleted());
      pendingDelete = null;
      await refresh();
    } catch (e) {
      toast.error((e as Error).message);
    } finally {
      deleting = false;
    }
  }
</script>

<div class="page">
  <div class="page-header">
    <h1 class="page-title">{m.nav_accounts()}</h1>
    <Button size="sm" onclick={() => connect()}>{m.acct_connect()}</Button>
  </div>

  {#if loading}
    <div class="loading"><Spinner size="sm" /> {m.common_loading()}</div>
  {:else if accounts.length === 0}
    <EmptyState title={m.acct_empty()} description={m.acct_empty_desc()} />
  {:else}
    <table>
      <thead>
        <tr>
          <th scope="col">{m.acct_provider()}</th>
          <th scope="col">{m.acct_label()}</th>
          <th scope="col">{m.acct_expires()}</th>
          <th scope="col">{m.acct_status()}</th>
          <th scope="col"><span class="sr-only">{m.common_actions()}</span></th>
        </tr>
      </thead>
      <tbody>
        {#each accounts as a (a.id)}
          <tr>
            <td><code>{a.provider}</code></td>
            <td class="label-cell">{a.label}</td>
            <td class="date-cell">{a.expires_at ? formatDate(a.expires_at) : m.acct_never()}</td>
            <td>
              <!-- Text, not color alone, carries the state. -->
              {#if a.status === 'needs_login'}
                <span class="badge warn">{m.acct_status_needs_login()}</span>
                {#if a.revoked_reason}
                  <p class="reason">{a.revoked_reason}</p>
                {/if}
              {:else}
                <span class="badge ok">{m.acct_status_active()}</span>
              {/if}
            </td>
            <td class="action-cell">
              <Button
                variant={a.status === 'needs_login' ? 'primary' : 'outline'}
                size="sm"
                onclick={() => connect(a.provider)}
                ariaLabel={`${m.acct_reauth()} ${a.label}`}
              >{m.acct_reauth()}</Button>
              <Button
                variant="danger"
                size="sm"
                onclick={() => (pendingDelete = a)}
                ariaLabel={`${m.acct_delete()} ${a.label}`}
              >{m.acct_delete()}</Button>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

<ConnectAccountModal bind:open={connectOpen} provider={connectProvider} onconnected={onConnected} />

<AlertDialog.Root open={pendingDelete !== null} onOpenChange={(v) => { if (!v) pendingDelete = null; }}>
  <AlertDialog.Portal>
    <AlertDialog.Overlay class="dialog-overlay" />
    <AlertDialog.Content class="dialog-content">
      <div class="confirm">
        <AlertDialog.Title class="dialog-title">{m.acct_delete_title()}</AlertDialog.Title>
        <AlertDialog.Description class="confirm-desc">
          {m.acct_delete_confirm({ label: pendingDelete?.label ?? '' })}
        </AlertDialog.Description>
        <div class="confirm-footer">
          <AlertDialog.Cancel class="confirm-btn outline">{m.common_cancel()}</AlertDialog.Cancel>
          <button type="button" class="confirm-btn danger" disabled={deleting} onclick={confirmDelete}>
            {m.acct_delete()}
          </button>
        </div>
      </div>
    </AlertDialog.Content>
  </AlertDialog.Portal>
</AlertDialog.Root>

<style>
  .page-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 1.5rem;
  }

  .page-header h1 {
    margin: 0;
  }

  .loading {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-3);
    font-size: 0.875rem;
    padding: 16px 0;
  }

  .label-cell {
    font-weight: 500;
    color: var(--text-1);
  }

  .date-cell {
    font-size: 0.8125rem;
    white-space: nowrap;
  }

  .action-cell {
    text-align: right;
    white-space: nowrap;
  }

  .action-cell :global(.btn + .btn) {
    margin-left: 6px;
  }

  .badge {
    display: inline-flex;
    padding: 2px 8px;
    border-radius: 9999px;
    font-size: 0.75rem;
    font-weight: 500;
  }

  .badge.ok {
    background: color-mix(in oklch, var(--success) 15%, transparent);
    color: var(--success);
  }

  .badge.warn {
    background: color-mix(in oklch, var(--warning) 15%, transparent);
    color: var(--warning);
  }

  .reason {
    margin: 4px 0 0;
    font-size: 0.75rem;
    color: var(--text-3);
    max-width: 280px;
    overflow-wrap: anywhere;
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }

  .confirm {
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  :global(.confirm-desc) {
    font-size: 0.875rem;
    color: var(--text-2);
    margin: 0;
  }

  .confirm-footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  :global(.confirm-btn) {
    border-radius: var(--radius-sm);
    cursor: pointer;
    font-size: 0.8125rem;
    font-weight: 500;
    padding: 0.4375rem 0.875rem;
    border: 1px solid transparent;
  }

  :global(.confirm-btn.outline) {
    background: transparent;
    color: var(--text-2);
    border-color: var(--border-strong);
  }

  :global(.confirm-btn.danger) {
    background: var(--danger);
    color: #fff;
  }

  :global(.confirm-btn:disabled) {
    opacity: 0.45;
    cursor: not-allowed;
  }
</style>
