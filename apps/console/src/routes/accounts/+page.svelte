<script lang="ts">
  import { onMount } from 'svelte';
  import { AlertDialog } from 'bits-ui';
  import { toast } from 'svelte-sonner';
  import { api } from '$lib/api.js';
  import type { Account, ConnectionOutcome, ConnectionSummary } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { AccountCredits, Badge, Button, Card, EmptyState, Spinner, StatusDot, UsageOverview } from '$lib/components/index.js';
  import { formatRelativeTime } from '$lib/format.js';
  import AccountRouting from './AccountRouting.svelte';
  import ConnectAccountModal from './ConnectAccountModal.svelte';

  let accounts = $state<Account[]>([]);
  let connections = $state<ConnectionSummary[]>([]);
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
      const [accountList, connectionList] = await Promise.all([api.listAccounts(), api.listConnections()]);
      accounts = accountList.items;
      connections = connectionList.items;
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

  function onConnected(account: Account, outcome: ConnectionOutcome) {
    if (outcome.connection_error) {
      toast.warning(m.acct_connection_failed({ error: outcome.connection_error }));
    } else if (outcome.connection_id) {
      toast.success(m.acct_connected_with_connection({ label: account.label, id: outcome.connection_id }));
    } else {
      toast.success(m.acct_connected({ label: account.label }));
    }
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
    <div class="page-actions">
      <Button onclick={() => connect()}>{m.acct_connect()}</Button>
    </div>
  </div>

  {#if loading}
    <div class="loading"><Spinner size="sm" /> {m.common_loading()}</div>
  {:else if accounts.length === 0}
    <EmptyState title={m.acct_empty()} description={m.acct_empty_desc()} />
  {:else}
    <div class="overview-slot"><UsageOverview {accounts} /></div>
    <div class="account-grid">
      {#each accounts as account (account.id)}
        <Card padding="0">
          <article class="account-card" class:needs-login={account.status === 'needs_login'}>
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

            <AccountRouting
              {account}
              connections={connections.filter((c) => c.account_id === account.id)}
              onchanged={refresh}
            />

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
  .loading {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-3);
    font-size: var(--text-base);
    padding: var(--space-4) 0;
  }

  .overview-slot { margin-bottom: var(--space-5); }

  .account-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, var(--col-lg)), 1fr));
    gap: var(--space-4);
    align-items: stretch;
  }

  .account-card {
    height: 100%;
    padding: var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-width: 0;
  }

  /* Credential needing attention: tint the edge, text still says why. */
  .account-card.needs-login {
    box-shadow: inset var(--indicator-w) 0 0 var(--warning);
    border-radius: var(--radius-lg);
  }

  .account-header,
  .account-actions {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
    flex-wrap: wrap;
  }

  .account-identity {
    min-width: 0;
    flex: 1 1 var(--col-sm);
  }

  .account-identity h2 {
    margin: var(--space-1) 0 0;
    color: var(--text-1);
    font-size: var(--text-md);
    font-weight: var(--weight-semibold);
    line-height: var(--leading-tight);
    overflow-wrap: anywhere;
  }

  .provider {
    display: inline-flex;
    align-items: center;
    padding: var(--space-0) var(--space-2);
    border: var(--border-w) solid var(--border);
    border-radius: var(--radius-full);
    background: var(--bg-elevated);
    color: var(--text-2);
    font-size: var(--text-2xs);
  }

  .account-status {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-shrink: 0;
  }

  /* Credential health strip: expiry + refresh-token presence. */
  .account-details {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    flex-wrap: wrap;
    padding: var(--space-3) var(--space-4);
    background: var(--bg-inset);
    border: var(--border-w) solid var(--border);
    border-radius: var(--radius);
  }

  .detail {
    display: flex;
    flex-direction: column;
    gap: var(--space-0);
    min-width: 0;
  }

  .detail-label {
    color: var(--text-3);
    font-size: var(--text-xs);
  }

  .detail-value {
    color: var(--text-1);
    font-size: var(--text-sm);
  }

  /* AccountCredits' panel variant is styled to bleed to a full-width strip
     (as on the provider detail page); cancel this card's own padding so it
     touches the card edges instead of floating as an inset box. */
  .credits-bleed {
    margin: 0 calc(var(--space-5) * -1);
  }

  .reason {
    margin: 0;
    padding: var(--space-3) var(--space-4);
    border-left: var(--focus-w) solid var(--warning);
    border-radius: var(--radius-sm);
    background: var(--warning-subtle);
    color: var(--text-1);
    font-size: var(--text-xs);
    line-height: var(--leading);
    overflow-wrap: anywhere;
  }

  .account-actions {
    justify-content: flex-end;
    align-items: center;
    margin-top: auto;
    padding-top: var(--space-4);
    border-top: var(--border-w) solid var(--border);
  }

  .account-actions :global(.btn + .btn) {
    margin-left: 0;
  }

  @media (max-width: 480px) {
    .account-actions { justify-content: stretch; }
    .account-actions :global(.btn) { flex: 1 1 auto; }
  }

  .confirm {
    padding: var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    overflow-y: auto;
  }

  :global(.confirm-desc) {
    font-size: var(--text-base);
    color: var(--text-2);
    margin: 0;
    line-height: var(--leading);
  }

  .confirm-footer {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  :global(.confirm-btn) {
    border-radius: var(--radius);
    cursor: pointer;
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    min-height: var(--control-h);
    padding: 0 var(--space-4);
    border: var(--border-w) solid transparent;
  }

  :global(.confirm-btn:focus-visible) {
    outline: var(--focus-w) solid var(--accent);
    outline-offset: var(--focus-w);
  }

  :global(.confirm-btn.outline) {
    background: transparent;
    color: var(--text-1);
    border-color: var(--border-strong);
  }

  :global(.confirm-btn.outline:hover) {
    background: var(--bg-hover);
  }

  :global(.confirm-btn.danger) {
    background: var(--danger);
    color: var(--on-accent);
  }

  :global(.confirm-btn.danger:hover:not(:disabled)) {
    background: color-mix(in oklch, var(--danger) 88%, var(--text-1));
  }

  :global(.confirm-btn:disabled) {
    opacity: 0.45;
    cursor: not-allowed;
  }
</style>
