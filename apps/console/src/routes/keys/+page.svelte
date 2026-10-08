<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api.js';
  import type { ClientKey, CreatedKey } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { EmptyState, Spinner } from '$lib/components/index.js';
  import { toast } from 'svelte-sonner';
  import ClaudeCodeSetup from './ClaudeCodeSetup.svelte';
  import CreateKeyForm from './CreateKeyForm.svelte';
  import EditKeyDialog from './EditKeyDialog.svelte';
  import KeyCard from './KeyCard.svelte';
  import RevealedSecret from './RevealedSecret.svelte';

  let keys = $state<ClientKey[]>([]);
  let loading = $state(true);
  let createdKey = $state<CreatedKey | null>(null);
  /** Raw secret from a regenerate; shown once, above the list. */
  let regenerated = $state<CreatedKey | null>(null);
  let expandedId = $state<string | null>(null);
  let busyId = $state<string | null>(null);
  let editing = $state<ClientKey | null>(null);
  let editOpen = $state(false);

  async function reload() {
    keys = (await api.listKeys()).items;
  }

  onMount(async () => {
    try {
      await reload();
    } catch (e) {
      toast.error((e as Error).message);
    } finally {
      loading = false;
    }
  });

  async function onCreated(result: CreatedKey) {
    createdKey = result;
    await reload();
  }

  function openEdit(k: ClientKey) {
    editing = k;
    editOpen = true;
  }

  async function regenerateKey(k: ClientKey) {
    if (!confirm(m.key_regenerate_confirm())) return;
    busyId = k.id;
    try {
      regenerated = await api.regenerateKey(k.id);
      await reload();
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
      await reload();
    } catch (e) {
      toast.error((e as Error).message);
    } finally {
      busyId = null;
    }
  }

  async function revokeKey(id: string) {
    if (!confirm(m.key_revoke_confirm())) return;
    try {
      await api.revokeKey(id);
      // Revoked keys stay listed so the operator can see what was cut off.
      await reload();
    } catch (e) {
      toast.error((e as Error).message);
    }
  }
</script>

<div class="page">
  <div class="page-header">
    <h1 class="page-title">{m.nav_keys()}</h1>
  </div>

  <div class="stack">
    {#if createdKey}
      <RevealedSecret secret={createdKey.key} notice={m.key_store_warning()} />
    {/if}
    {#if regenerated}
      <RevealedSecret secret={regenerated.key} notice={`${m.key_regenerated_notice({ name: regenerated.name })}. ${m.key_store_warning()}`} />
    {/if}

    <section aria-labelledby="keys-heading" class="panel">
      <div class="panel-head">
        <h2 id="keys-heading">{m.nav_keys()}</h2>
        <span class="count">{keys.length}</span>
      </div>
      <div class="panel-body">
        {#if loading}
          <div class="loading"><Spinner size="sm" /> {m.common_loading()}</div>
        {:else if keys.length === 0}
          <EmptyState title={m.key_empty()} description={m.key_empty_desc()} />
        {:else}
          <ul class="key-list">
            {#each keys as k (k.id)}
              <KeyCard
                {k}
                expanded={expandedId === k.id}
                busy={busyId === k.id}
                ontoggle={() => (expandedId = expandedId === k.id ? null : k.id)}
                onedit={() => openEdit(k)}
                onregenerate={() => regenerateKey(k)}
                ontoggledisabled={() => toggleDisabled(k)}
                onrevoke={() => revokeKey(k.id)}
              />
            {/each}
          </ul>
        {/if}
      </div>
    </section>

    <CreateKeyForm onstart={() => (createdKey = null)} oncreated={onCreated} />

    <ClaudeCodeSetup />
  </div>
</div>

<EditKeyDialog bind:open={editOpen} {editing} onsaved={reload} />

<style>
  .loading {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-3);
    font-size: var(--text-base);
    padding: var(--space-4) 0;
  }

  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }

  .stack > :global(section) { margin-bottom: 0; }

  .count {
    color: var(--text-3);
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
  }

  .key-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, var(--col-xl)), 1fr));
    gap: var(--space-4);
    align-items: start;
  }
</style>
