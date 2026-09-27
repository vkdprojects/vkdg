<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api.js';
  import type { PluginSummary } from '$lib/api.js';
  import { Badge, EmptyState, Button, Spinner } from '$lib/components/index.js';
  import { toast } from 'svelte-sonner';

  let plugins = $state<PluginSummary[]>([]);
  let directory = $state('');
  let loading = $state(true);
  let installing = $state(false);

  let dialogEl = $state<HTMLDialogElement | null>(null);
  let manifest = $state('');
  let wasmBase64 = $state<string | undefined>(undefined);
  let wasmName = $state('');
  let formError = $state('');

  async function refresh() {
    const res = await api.listPlugins();
    plugins = res.items;
    directory = res.directory;
  }

  onMount(async () => {
    try {
      await refresh();
    } catch (e) {
      toast.error((e as Error).message);
    } finally {
      loading = false;
    }
  });

  function openDialog() {
    manifest = '';
    wasmBase64 = undefined;
    wasmName = '';
    formError = '';
    dialogEl?.showModal();
  }

  function closeDialog() {
    dialogEl?.close();
  }

  /** Read a picked .wasm as base64, which is how the install endpoint takes bytes. */
  async function onWasmPicked(e: Event) {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) {
      wasmBase64 = undefined;
      wasmName = '';
      return;
    }
    const buffer = await file.arrayBuffer();
    const bytes = new Uint8Array(buffer);
    // Chunked to avoid blowing the argument limit on a large component.
    let binary = '';
    const chunk = 0x8000;
    for (let i = 0; i < bytes.length; i += chunk) {
      binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
    }
    wasmBase64 = btoa(binary);
    wasmName = file.name;
  }

  async function install(e: Event) {
    e.preventDefault();
    formError = '';
    if (!manifest.trim()) {
      formError = 'Paste the plugin manifest YAML.';
      return;
    }
    installing = true;
    try {
      const res = await api.installPlugin(manifest, wasmBase64);
      toast.success(`Installed ${res.name} ${res.version}`);
      closeDialog();
      await refresh();
    } catch (err) {
      // The gateway validates the manifest and verifies the checksum, so its
      // message is the useful one to show.
      formError = (err as Error).message;
    } finally {
      installing = false;
    }
  }

  async function remove(name: string) {
    try {
      await api.removePlugin(name);
      toast.success(`Removed ${name}`);
      await refresh();
    } catch (err) {
      toast.error((err as Error).message);
    }
  }
</script>

<div class="page">
  <div class="page-header">
    <h1 class="page-title">Plugins ({plugins.length})</h1>
    <Button onclick={openDialog}>Install plugin</Button>
  </div>

  {#if loading}
    <div class="loading"><Spinner size="sm" /> Loading…</div>
  {:else if plugins.length === 0}
    <EmptyState
      title="No plugins installed"
      description="Providers, compressors and routing strategies can be added without rebuilding the gateway."
    />
  {:else}
    <table>
      <thead>
        <tr>
          <th scope="col">Name</th>
          <th scope="col">Version</th>
          <th scope="col">Kind</th>
          <th scope="col">Models</th>
          <th scope="col">Tags</th>
          <th scope="col"><span class="sr-only">Actions</span></th>
        </tr>
      </thead>
      <tbody>
        {#each plugins as p (p.name)}
          <tr>
            <td>
              <div>{p.name}</div>
              <div class="muted">{p.description}</div>
            </td>
            <td>{p.version}</td>
            <td>{p.kind}</td>
            <td>{p.models.join(', ') || '—'}</td>
            <td class="badges">
              {#each p.tags as tag (tag)}<Badge status="cache" label={tag} />{/each}
              {#if p.tags.length === 0}<span class="muted">—</span>{/if}
            </td>
            <td>
              {#if p.removable}
                <Button variant="ghost" onclick={() => remove(p.name)}>Remove</Button>
              {/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}

  {#if directory}
    <p class="muted dir">Installed to {directory}</p>
  {/if}
</div>

<dialog bind:this={dialogEl} class="modal" aria-labelledby="install-title">
  <div class="modal-header">
    <h2 id="install-title">Install plugin</h2>
  </div>
  <form onsubmit={install}>
    <label for="manifest">Manifest YAML</label>
    <textarea
      id="manifest"
      bind:value={manifest}
      rows="12"
      placeholder={`name: my-provider\nversion: "1.0.0"\nkind: provider\ndescription: "..."\nlicense: MIT\ninstall:\n  wasm: "https://..."\n  checksum: "sha256:..."`}
    ></textarea>

    <label for="wasm">Component (.wasm) — only for a wasm plugin</label>
    <input id="wasm" type="file" accept=".wasm" onchange={onWasmPicked} />
    {#if wasmName}<p class="muted">Selected {wasmName}</p>{/if}

    {#if formError}<p class="error">{formError}</p>{/if}

    <div class="modal-actions">
      <Button type="button" variant="ghost" onclick={closeDialog}>Cancel</Button>
      <Button type="submit" disabled={installing}>
        {installing ? 'Installing…' : 'Install'}
      </Button>
    </div>
  </form>
</dialog>

<style>
  .page {
    padding: 1.5rem;
  }
  .page-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 1rem;
  }
  .page-title {
    font-size: 1.25rem;
    font-weight: 600;
  }
  .loading {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th,
  td {
    text-align: left;
    padding: 0.625rem 0.75rem;
    border-bottom: 1px solid var(--border, #2a2a2a);
    vertical-align: top;
  }
  .muted {
    color: var(--text-muted, #888);
    font-size: 0.875rem;
  }
  .dir {
    margin-top: 1rem;
  }
  .badges {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
  .modal {
    border: 1px solid var(--border, #2a2a2a);
    border-radius: 0.5rem;
    padding: 1.25rem;
    min-width: 32rem;
    max-width: 90vw;
  }
  .modal-header {
    margin-bottom: 0.75rem;
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  textarea {
    font-family: var(--font-mono, monospace);
    font-size: 0.8125rem;
    width: 100%;
  }
  .error {
    color: var(--danger, #e5484d);
    font-size: 0.875rem;
  }
  .modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
    margin-top: 0.75rem;
  }
</style>
