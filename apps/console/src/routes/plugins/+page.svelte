<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api.js';
  import type { PluginSummary } from '$lib/api.js';
  import { Badge, Card, EmptyState, Button, Spinner } from '$lib/components/index.js';
  import { toast } from 'svelte-sonner';
  import { m } from '$lib/paraglide/messages.js';

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
    <h1 class="page-title">{m.nav_plugins()} <span class="mono count">({plugins.length})</span></h1>
    <Button onclick={openDialog}>{m.plugin_install()}</Button>
  </div>

  {#if loading}
    <div class="loading"><Spinner size="sm" /> {m.common_loading()}</div>
  {:else if plugins.length === 0}
    <EmptyState
      title={m.plugin_empty()}
      description={m.plugin_empty_desc()}
    />
  {:else}
    <Card padding="0">
      <table>
        <thead>
          <tr>
            <th scope="col">{m.plugin_name()}</th>
            <th scope="col">{m.plugin_version()}</th>
            <th scope="col">{m.plugin_kind()}</th>
            <th scope="col">{m.plugin_models()}</th>
            <th scope="col">{m.plugin_tags()}</th>
            <th scope="col"><span class="sr-only">{m.common_actions()}</span></th>
          </tr>
        </thead>
        <tbody>
          {#each plugins as p (p.name)}
            <tr>
              <td>
                <div class="plugin-name mono">{p.name}</div>
                <div class="muted">{p.description}</div>
              </td>
              <td class="mono">{p.version}</td>
              <td class="mono">{p.kind}</td>
              <td class="muted">{p.models.join(', ') || m.common_none()}</td>
              <td class="badges">
                {#each p.tags as tag (tag)}<Badge status="cooldown" label={tag} />{/each}
                {#if p.tags.length === 0}<span class="muted">{m.common_none()}</span>{/if}
              </td>
              <td>
                {#if p.removable}
                  <Button variant="ghost" size="sm" onclick={() => remove(p.name)} ariaLabel={`${m.common_delete()} ${p.name}`}>{m.common_delete()}</Button>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </Card>
  {/if}

  {#if directory}
    <p class="muted dir mono">{m.plugin_installed_to({ directory })}</p>
  {/if}
</div>

<dialog bind:this={dialogEl} class="modal" aria-labelledby="install-title">
  <div class="modal-header">
    <h2 id="install-title">{m.plugin_install()}</h2>
    <button class="close-btn" onclick={closeDialog} aria-label={m.common_cancel()}>✕</button>
  </div>
  <form onsubmit={install} class="modal-form">
    <div class="field">
      <label for="manifest">{m.plugin_manifest_label()}</label>
      <textarea
        id="manifest"
        bind:value={manifest}
        rows="12"
        placeholder={`name: my-provider\nversion: "1.0.0"\nkind: provider\ndescription: "..."\nlicense: MIT\ninstall:\n  wasm: "https://..."\n  checksum: "sha256:..."`}
      ></textarea>
    </div>

    <div class="field">
      <label for="wasm">{m.plugin_wasm_label()}</label>
      <input id="wasm" type="file" accept=".wasm" onchange={onWasmPicked} />
      {#if wasmName}<p class="muted mono">{m.plugin_wasm_selected({ name: wasmName })}</p>{/if}
    </div>

    {#if formError}<p class="form-error" role="alert">{formError}</p>{/if}

    <div class="modal-actions">
      <Button type="button" variant="ghost" onclick={closeDialog}>{m.common_cancel()}</Button>
      <Button type="submit" disabled={installing}>
        {installing ? m.plugin_installing() : m.common_create()}
      </Button>
    </div>
  </form>
</dialog>

<style>
  .page-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 1.5rem;
  }
  .page-title {
    margin: 0;
  }
  .count {
    color: var(--text-3);
    font-weight: 400;
  }
  .loading {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-3);
    font-size: var(--text-sm);
    padding: 32px 0;
  }
  table {
    margin: 0;
  }
  th:first-child,
  td:first-child {
    padding-left: 1.25rem;
  }
  th:last-child,
  td:last-child {
    padding-right: 1.25rem;
  }
  .plugin-name {
    color: var(--text-1);
    font-weight: 600;
    font-size: var(--text-sm);
  }
  .muted {
    color: var(--text-3);
    font-size: var(--text-sm);
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
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text-1);
    padding: 0;
    width: min(560px, calc(100vw - 32px));
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.32);
  }
  .modal::backdrop {
    background: rgba(0, 0, 0, 0.5);
  }
  .modal-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 20px 24px 0;
  }
  .modal-header h2 {
    font-size: 1rem;
    font-weight: 600;
    margin: 0;
  }
  .close-btn {
    background: transparent;
    border: none;
    color: var(--text-3);
    cursor: pointer;
    font-size: 1rem;
    line-height: 1;
    padding: 4px;
    border-radius: var(--radius-sm);
    transition: color 0.1s;
  }
  .close-btn:hover {
    color: var(--text-1);
  }
  .modal-form {
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 20px 24px 24px;
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
  textarea {
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--text-1);
    font-family: var(--font-mono);
    font-size: 0.8125rem;
    padding: 0.4375rem 0.625rem;
    width: 100%;
    box-sizing: border-box;
    transition: border-color 0.15s;
  }
  textarea:focus {
    border-color: var(--accent);
    outline: none;
  }
  input[type='file'] {
    font-size: 0.8125rem;
    color: var(--text-2);
  }
  .form-error {
    font-size: 0.8125rem;
    color: var(--danger);
    margin: 0;
  }
  .modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }
</style>
