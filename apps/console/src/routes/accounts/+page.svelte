<script lang="ts">
  import { onMount } from 'svelte';
  import { AlertDialog } from 'bits-ui';
  import { toast } from 'svelte-sonner';
  import { api } from '$lib/api.js';
  import type { Account } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { AccountCredits, Badge, Button, Card, EmptyState, Spinner, StatusDot } from '$lib/components/index.js';
  import { formatRelativeTime } from '$lib/format.js';
  import ConnectAccountModal from './ConnectAccountModal.svelte';

  let accounts = $state<Account[]>([]);
  let loading = $state(true);
  let connectOpen = $state(false);
  let connectProvider = $state('kiro');
  let connectAccountId = $state<string | undefined>(undefined);
  let pendingDelete = $state<Account | null>(null);
  let deleting = $state(false);

  function formatExpiry(iso: string): string {
    const date = new Date(iso);
    const seconds = Math.round((date.getTime() - Date.now()) / 1000);

    if (Math.abs(seconds) < 60) return formatRelativeTime(seconds, 'second');
    const minutes = Math.round(seconds / 60);
    if (Math.abs(minutes) < 60) return formatRelativeTime(minutes, 'minute');
    const hours = Math.round(minutes / 60);
    if (Math.abs(hours) < 24) return formatRelativeTime(hours, 'hour');
    const days = Math.round(hours / 24);
    if (Math.abs(days) < 30) return formatRelativeTime(days, 'day');
    const months = Math.round(days / 30);
    if (Math.abs(months) < 12) return formatRelativeTime(months, 'month');
    return formatRelativeTime(Math.round(months / 12), 'year');
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

  /** New account, or with `accountId` a reconnect of that account. */
  function connect(provider = 'kiro', accountId?: string) {
    connectProvider = provider;
    connectAccountId = accountId;
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
    <div class="account-grid">
      {#each accounts as account (account.id)}
        <Card padding="0">
          <article class="account-card">
            <header class="account-header">
              <div class="account-identity">
                <span class="provider mono">{account.provider}</span>
                <h2>{account.label}</h2>
              </div>
              <div class="account-status">
                <StatusDot status={account.status === 'active' ? 'healthy' : 'degraded'} />
                <Badge
                  status={account.status === 'active' ? 'healthy' : 'degraded'}
                  label={account.status === 'active' ? m.acct_status_active() : m.acct_status_needs_login()}
                />
              </div>
            </header>

            <div class="account-details">
              <div class="detail">
                <span class="detail-label">{m.acct_expires()}</span>
                <span class="detail-value mono" title={account.expires_at ?? undefined}>
                  {account.expires_at ? formatExpiry(account.expires_at) : m.acct_never()}
                </span>
              </div>
              {#if account.has_refresh_token}
                <Badge status="cancelled" label={m.acct_refresh_token()} />
              {/if}
            </div>

            <div class="credits-bleed"><AccountCredits account={account} /></div>

            {#if account.revoked_reason}
              <p class="reason">{account.revoked_reason}</p>
            {/if}

            <footer class="account-actions">
              <Button
                variant={account.status === 'needs_login' ? 'primary' : 'outline'}
                size="sm"
                onclick={() => connect(account.provider, account.id)}
                ariaLabel={`${m.acct_reauth()} ${account.label}`}
              >{m.acct_reauth()}</Button>
              <Button
                variant="danger"
                size="sm"
                onclick={() => (pendingDelete = account)}
                ariaLabel={`${m.acct_delete()} ${account.label}`}
              >{m.acct_delete()}</Button>
            </footer>
          </article>
        </Card>
      {/each}
    </div>
  {/if}
</div>

<ConnectAccountModal bind:open={connectOpen} provider={connectProvider} accountId={connectAccountId} onconnected={onConnected} />

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

  .account-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, 320px), 1fr));
    gap: 12px;
  }

  .account-card {
    min-height: 100%;
    padding: 1rem;
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  .account-header,
  .account-details,
  .account-actions {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .account-identity {
    min-width: 0;
  }

  .account-identity h2 {
    margin: 3px 0 0;
    color: var(--text-1);
    font-size: 0.9375rem;
    line-height: 1.25;
    overflow-wrap: anywhere;
  }

  .provider,
  .detail-label {
    color: var(--text-3);
    font-size: var(--text-2xs);
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  .account-status {
    display: flex;
    align-items: center;
    gap: 7px;
    flex-shrink: 0;
  }

  .account-details {
    padding-top: 0.75rem;
    border-top: 1px solid var(--border);
  }

  /* AccountCredits' panel variant is styled to bleed to a full-width strip
     (as on the provider detail page); cancel this card's own padding so it
     touches the card edges instead of floating as an inset box. */
  .credits-bleed {
    margin: 0 -1rem;
  }

  .detail {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .detail-value {
    color: var(--text-2);
    font-size: 0.8125rem;
  }

  .reason {
    margin: -0.25rem 0 0;
    padding: 0.625rem 0.75rem;
    border-left: 2px solid var(--warning);
    background: color-mix(in oklch, var(--warning) 7%, transparent);
    color: var(--text-2);
    font-size: 0.75rem;
    line-height: 1.45;
    overflow-wrap: anywhere;
  }

  .account-actions {
    justify-content: flex-end;
    margin-top: auto;
  }

  .account-actions :global(.btn + .btn) {
    margin-left: 0;
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
