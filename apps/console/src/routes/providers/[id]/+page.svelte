<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import { ExternalLink } from 'lucide-svelte';
  import { api } from '$lib/api.js';
  import type { Account, OAuthProvider } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { Button, EmptyState, Spinner } from '$lib/components/index.js';
  import ConnectAccountModal from '../../accounts/ConnectAccountModal.svelte';

  const id = $derived(decodeURIComponent($page.params['id'] ?? ''));
  let provider = $state<OAuthProvider | null>(null);
  let accounts = $state<Account[]>([]);
  let loading = $state(true);
  let connectOpen = $state(false);
  let connectAccountId = $state<string | null>(null);
  let deleting = $state<string | null>(null);

  function formatDate(iso: string): string {
    return new Intl.DateTimeFormat(undefined, {
      year: 'numeric', month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit',
    }).format(new Date(iso));
  }

  async function refresh() {
    const [pRes, aRes] = await Promise.all([api.oauthProviders(), api.listAccounts()]);
    provider = pRes.items.find((p) => p.id === id) ?? null;
    accounts = aRes.items.filter((a) => a.provider === id);
    if (!provider) goto('/providers');
  }

  onMount(async () => {
    try { await refresh(); } finally { loading = false; }
  });

  function connect(accountId?: string) {
    connectAccountId = accountId ?? null;
    connectOpen = true;
  }

  async function deleteAccount(a: Account) {
    if (!confirm(m.acct_delete_confirm({ label: a.label }))) return;
    deleting = a.id;
    try {
      await api.deleteAccount(a.id);
      await refresh();
    } finally {
      deleting = null;
    }
  }

  function onConnected(_a: Account) {
    connectOpen = false;
    refresh();
  }
</script>

<div class="page">
  {#if loading}
    <div class="loading"><Spinner size="sm" /> {m.common_loading()}</div>
  {:else if provider}
    <div class="header">
      <a class="back" href="/providers">← {m.providers_title()}</a>
      <div class="provider-head">
        <span class="icon" style:background={provider.icon_color}>{provider.icon_char}</span>
        <div>
          <div class="name-row">
            <h1 class="page-title">{provider.display_name}</h1>
            {#if provider.site_url}
              <a class="site-link" href={provider.site_url} target="_blank" rel="noopener noreferrer"
                aria-label="Open {provider.display_name} website">
                <ExternalLink size={14} aria-hidden="true" />
              </a>
            {/if}
          </div>
          {#if provider.description}
            <p class="desc">{provider.description}</p>
          {/if}
        </div>
      </div>
    </div>

    <div class="section-header">
      <h2 class="section-title">{m.provider_detail_accounts()}</h2>
      <Button size="sm" onclick={() => connect()}>{m.provider_detail_connect()}</Button>
    </div>

    {#if accounts.length === 0}
      <EmptyState title={m.provider_detail_no_accounts()} description={m.providers_connect_first()} />
    {:else}
      <table>
        <thead>
          <tr>
            <th scope="col">{m.acct_label()}</th>
            <th scope="col">{m.acct_expires()}</th>
            <th scope="col">{m.acct_status()}</th>
            <th scope="col"><span class="sr-only">{m.common_actions()}</span></th>
          </tr>
        </thead>
        <tbody>
          {#each accounts as a (a.id)}
            <tr>
              <td class="label-cell">{a.label}</td>
              <td class="date-cell">{a.expires_at ? formatDate(a.expires_at) : m.acct_never()}</td>
              <td>
                {#if a.status === 'needs_login'}
                  <span class="badge warn">{m.acct_status_needs_login()}</span>
                {:else}
                  <span class="badge ok">{m.acct_status_active()}</span>
                {/if}
              </td>
              <td class="action-cell">
                <Button variant={a.status === 'needs_login' ? 'primary' : 'outline'} size="sm"
                  onclick={() => connect(a.id)}>{m.acct_reauth()}</Button>
                <Button variant="danger" size="sm" disabled={deleting === a.id}
                  onclick={() => deleteAccount(a)}>{m.acct_delete()}</Button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  {/if}
</div>

{#if provider}
  <ConnectAccountModal
    bind:open={connectOpen}
    provider={id}
    accountId={connectAccountId ?? undefined}
    onconnected={onConnected}
  />
{/if}

<style>
  .loading { display: flex; align-items: center; gap: 8px; color: var(--text-3); font-size: 0.875rem; padding: 16px 0; }
  .back { color: var(--text-3); font-size: 0.875rem; text-decoration: none; display: inline-flex; align-items: center; gap: 4px; margin-bottom: 1rem; }
  .back:hover { color: var(--accent); }
  .header { margin-bottom: 2rem; }
  .provider-head { display: flex; align-items: flex-start; gap: 16px; margin-top: 0.75rem; }
  .icon { align-items: center; border-radius: 12px; color: #fff; display: flex; flex-shrink: 0; font-size: 1.5rem; font-weight: 700; height: 56px; justify-content: center; width: 56px; }
  .name-row { display: flex; align-items: center; gap: 8px; }
  .page-title { margin: 0; }
  .site-link { color: var(--text-3); display: flex; }
  .site-link:hover { color: var(--accent); }
  .desc { color: var(--text-3); font-size: 0.875rem; margin: 4px 0 0; }
  .section-header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 1rem; }
  .section-title { font-size: 1rem; font-weight: 600; color: var(--text-1); margin: 0; }
  .label-cell { font-weight: 500; color: var(--text-1); }
  .date-cell { font-size: 0.8125rem; white-space: nowrap; }
  .action-cell { text-align: right; white-space: nowrap; }
  .action-cell :global(.btn + .btn) { margin-left: 6px; }
  .badge { display: inline-flex; padding: 2px 8px; border-radius: 9999px; font-size: 0.75rem; font-weight: 500; }
  .badge.ok { background: color-mix(in oklch, var(--success) 15%, transparent); color: var(--success); }
  .badge.warn { background: color-mix(in oklch, var(--warning) 15%, transparent); color: var(--warning); }
  .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; }
</style>
