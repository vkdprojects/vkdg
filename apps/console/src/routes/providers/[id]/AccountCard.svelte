<script lang="ts">
  import { RefreshCwIcon, Trash2Icon } from 'lucide-svelte';
  import type { Account, ConnectionSummary } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { formatDateTime, formatRelativeFrom } from '$lib/format.js';
  import { AccountCredits, Badge, Button } from '$lib/components/index.js';
  import { clock } from '$lib/live.svelte.js';
  import AccountRouting from '../../accounts/AccountRouting.svelte';

  interface Props {
    account: Account;
    connections: ConnectionSummary[];
    /** Same upstream user resolved under more than one local account. */
    duplicateUser: boolean;
    deleting: boolean;
    onConnect: (accountId: string) => void;
    onDelete: (account: Account) => void;
    onChanged: () => void;
  }

  let { account: a, connections, duplicateUser, deleting, onConnect, onDelete, onChanged }: Props = $props();

  // Coarse "in 5 minutes" / "2 hours ago": 30s resolution is plenty.
  const ticker = clock(30_000);

  const expired = $derived(!!a.expires_at && new Date(a.expires_at).getTime() <= ticker.now);
</script>

<article class="account glass">
  <header class="account-head">
    <div class="account-title">
      <span class="eyebrow">{m.provider_account()}</span>
      <h3>{a.label}</h3>
      <span class="mono account-id">{a.id}</span>
    </div>
    <Badge status={a.status === 'active' ? 'healthy' : 'degraded'} label={a.status === 'active' ? m.acct_status_active() : m.acct_status_needs_login()} />
  </header>

  <div class="account-chips">
    {#if a.expires_at}
      <span class="chip" class:chip-danger={expired} title={formatDateTime(a.expires_at)}>
        {expired ? m.pd_token_expired() : m.pd_token_expires_in({ when: formatRelativeFrom(a.expires_at, m.common_none(), ticker.now) })}
      </span>
    {:else}
      <span class="chip">{m.pd_token_never()}</span>
    {/if}
    <span class="chip" class:accent={a.has_refresh_token}>{a.has_refresh_token ? m.pd_refresh_token_yes() : m.pd_refresh_token_no()}</span>
    {#if a.revoked_reason}<span class="chip chip-danger" title={m.account_revocation_reason()}>{a.revoked_reason}</span>{/if}
    {#if a.credits_user_ref && duplicateUser}
      <span class="dup-warning" title={m.account_duplicate_user_desc()}><Badge status="degraded" label={m.account_duplicate_user()} /></span>
    {/if}
  </div>

  <AccountCredits account={a} />

  <div class="connections-block"><AccountRouting account={a} {connections} onchanged={onChanged} /></div>

  <footer class="actions-row account-actions">
    <Button
      variant={a.status === 'needs_login' ? 'primary' : 'outline'}
      size="sm"
      onclick={() => onConnect(a.id)}
      ariaLabel={m.pd_account_reconnect_label({ label: a.label })}
    >
      <RefreshCwIcon size={14} aria-hidden="true" /> {m.acct_reauth()}
    </Button>
    <Button
      variant="danger"
      size="sm"
      disabled={deleting}
      onclick={() => onDelete(a)}
      ariaLabel={m.pd_account_delete_label({ label: a.label })}
    >
      <Trash2Icon size={14} aria-hidden="true" /> {m.acct_delete()}
    </Button>
  </footer>
</article>

<style>
  .account { display: flex; flex-direction: column; gap: var(--space-4); padding: var(--space-5); min-width: 0; }
  .account-head { display: flex; align-items: flex-start; justify-content: space-between; gap: var(--space-3); flex-wrap: wrap; }
  .account-title { min-width: 0; }
  .account-title h3 { margin: var(--space-0) 0; font-size: var(--text-md); overflow-wrap: anywhere; }
  .eyebrow { color: var(--text-3); font-size: var(--text-xs); font-weight: var(--weight-medium); }
  .account-id { color: var(--text-3); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .account-chips { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2); }
  .chip-danger { color: var(--danger); background: var(--danger-subtle); border-color: color-mix(in oklch, var(--danger) 35%, transparent); }
  .dup-warning { display: inline-flex; align-items: center; }
  .connections-block { min-width: 0; }
  .account-actions { justify-content: flex-end; padding-top: var(--space-3); border-top: var(--border-w) solid var(--border); }

  @media (max-width: 480px) {
    .account { padding: var(--space-4); }
    .account-actions > :global(*) { flex: 1 1 auto; }
  }
</style>
