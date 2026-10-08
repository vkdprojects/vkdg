<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api.js';
  import type { PluginSummary } from '$lib/api.js';
  import { Badge, EmptyState, Button, PageCount, SurfaceCard } from '$lib/components/index.js';
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
    <div>
      <h1>{m.nav_plugins()} <PageCount value={plugins.length} /></h1>
      {#if directory}
        <p class="dir mono">{m.plugin_installed_to({ directory })}</p>
      {/if}
    </div>
    <div class="page-actions">
      <Button onclick={openDialog}>{m.plugin_install()}</Button>
    </div>
  </div>

  {#if loading}
    <div class="plugin-grid" aria-busy="true">
      <div class="skeleton plugin-skel"></div>
      <div class="skeleton plugin-skel"></div>
      <div class="skeleton plugin-skel"></div>
    </div>
  {:else if plugins.length === 0}
    <EmptyState
      title={m.plugin_empty()}
      description={m.plugin_empty_desc()}
    />
  {:else}
    <div class="plugin-grid">
      {#each plugins as p (p.name)}
        <SurfaceCard>
          <header class="plugin-head">
            <h3 class="plugin-name mono">{p.name}</h3>
            <span class="plugin-version mono">{p.version}</span>
          </header>
          {#if p.description}<p class="plugin-desc">{p.description}</p>{/if}

          <dl class="plugin-meta">
            <div>
              <dt>{m.plugin_kind()}</dt>
              <dd class="mono">{p.kind}</dd>
            </div>
            <div>
              <dt>{m.plugin_models()}</dt>
              <dd class="models">{p.models.join(', ') || m.common_none()}</dd>
            </div>
            <div>
              <dt>{m.plugin_tags()}</dt>
              <dd class="badges">
                {#each p.tags as tag (tag)}<Badge status="cooldown" label={tag} />{/each}
                {#if p.tags.length === 0}<span class="muted">{m.common_none()}</span>{/if}
              </dd>
            </div>
          </dl>

          {#if p.removable}
            <footer class="plugin-foot">
              <Button variant="ghost" size="sm" onclick={() => remove(p.name)} ariaLabel={`${m.common_delete()} ${p.name}`}>{m.common_delete()}</Button>
            </footer>
          {/if}
        </SurfaceCard>
      {/each}
    </div>
  {/if}
</div>

<dialog bind:this={dialogEl} class="dialog-content" aria-labelledby="install-title">
  <div class="dialog-header">
    <h2 id="install-title" class="dialog-title">{m.plugin_install()}</h2>
    <button class="dialog-close" onclick={closeDialog} aria-label={m.common_cancel()}>✕</button>
  </div>
  <form onsubmit={install}>
   <div class="dialog-body fields">
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
   </div>

    <div class="dialog-footer">
      <Button type="button" variant="ghost" onclick={closeDialog}>{m.common_cancel()}</Button>
      <Button type="submit" disabled={installing}>
        {installing ? m.plugin_installing() : m.common_create()}
      </Button>
    </div>
  </form>
</dialog>

<style>
  .dir {
    color: var(--text-3);
    overflow-wrap: anywhere;
  }

  .plugin-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, var(--col-lg)), 1fr));
    gap: var(--space-4);
  }

  .plugin-skel {
    height: var(--col-sm);
    border-radius: var(--radius-lg);
  }

  .plugin-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .plugin-name {
    margin: 0;
    font-size: var(--text-base);
    font-weight: var(--weight-semibold);
    color: var(--text-1);
    overflow-wrap: anywhere;
  }

  .plugin-version {
    flex: none;
    color: var(--text-3);
  }

  .plugin-desc {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-2);
  }

  .plugin-meta {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding-top: var(--space-3);
    border-top: var(--border-w) solid var(--border);
    font-size: var(--text-sm);
  }

  .plugin-meta > div {
    display: grid;
    grid-template-columns: var(--col-xs) minmax(0, 1fr);
    gap: var(--space-3);
    align-items: baseline;
  }

  .plugin-meta dt {
    color: var(--text-3);
    font-size: var(--text-xs);
  }

  .plugin-meta dd {
    margin: 0;
    color: var(--text-2);
    overflow-wrap: anywhere;
  }

  .models {
    color: var(--text-2);
  }

  .badges {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }

  .plugin-foot {
    display: flex;
    justify-content: flex-end;
    margin-top: auto;
    padding-top: var(--space-1);
  }

  /* native <dialog>: global .dialog-content supplies chrome; reset UA defaults + style the backdrop */
  dialog.dialog-content {
    padding: 0;
    margin: 0;
    color: var(--text-1);
    width: var(--dialog-w-lg);
  }

  dialog.dialog-content:not([open]) {
    display: none;
  }

  dialog.dialog-content::backdrop {
    background: var(--bg-scrim);
    backdrop-filter: var(--glass-blur);
  }

  .dialog-content form {
    display: flex;
    flex-direction: column;
    min-height: 0;
    flex: 1;
  }

  textarea {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    resize: vertical;
    min-height: var(--col-xs);
  }

  input[type='file'] {
    font-size: var(--text-sm);
    color: var(--text-2);
    padding: var(--space-2);
  }

  .form-error {
    font-size: var(--text-sm);
    color: var(--danger);
    margin: 0;
  }
</style>
