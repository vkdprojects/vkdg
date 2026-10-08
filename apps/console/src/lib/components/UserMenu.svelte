<script lang="ts">
  import { DropdownMenu } from 'bits-ui';
  import { Check, ChevronsUpDown, LogOut, Moon, Sun } from 'lucide-svelte';
  import { m } from '$lib/paraglide/messages.js';
  import { getLocale, setLocale } from '$lib/paraglide/runtime.js';

  interface Props {
    role: string;
    theme: 'dark' | 'light';
    onThemeChange: (t: 'dark' | 'light') => void;
    onSignOut: () => void;
    /** Icon-rail mode: avatar only. */
    compact?: boolean;
  }
  let { role, theme, onThemeChange, onSignOut, compact = false }: Props = $props();

  // Endonyms on purpose: a language is always listed in its own name.
  const locales = [
    { id: 'en', name: 'English', flag: '/flags/us.svg' },
    { id: 'pt-BR', name: 'Português (Brasil)', flag: '/flags/br.svg' },
  ] as const;
  const current = $derived(locales.find((l) => l.id === getLocale()) ?? locales[0]);
</script>

<DropdownMenu.Root>
  <DropdownMenu.Trigger class="um-trigger" aria-label={m.shell_account_menu()}>
    <span class="um-avatar" aria-hidden="true">{role.slice(0, 1).toUpperCase()}</span>
    {#if !compact}
      <span class="um-who">
        <span class="um-role">{role}</span>
        <span class="um-lang"><img src={current.flag} alt="" class="flag" />{current.name}</span>
      </span>
      <ChevronsUpDown size={14} aria-hidden="true" />
    {/if}
  </DropdownMenu.Trigger>

  <DropdownMenu.Portal>
    <DropdownMenu.Content class="um-content" side="top" align="start" sideOffset={8}>
      <DropdownMenu.Group>
        <DropdownMenu.GroupHeading class="um-heading">{m.shell_language()}</DropdownMenu.GroupHeading>
        {#each locales as l (l.id)}
          <DropdownMenu.Item class="um-item" onSelect={() => setLocale(l.id)}>
            <img src={l.flag} alt="" class="flag" />
            <span class="um-label">{l.name}</span>
            {#if l.id === current.id}<Check size={14} aria-hidden="true" />{/if}
          </DropdownMenu.Item>
        {/each}
      </DropdownMenu.Group>

      <DropdownMenu.Separator class="um-sep" />

      <DropdownMenu.Group>
        <DropdownMenu.GroupHeading class="um-heading">{m.shell_theme()}</DropdownMenu.GroupHeading>
        <div class="um-theme segmented" role="group" aria-label={m.shell_theme()}>
          <button type="button" aria-pressed={theme === 'dark'} onclick={() => onThemeChange('dark')}><Moon size={14} aria-hidden="true" />{m.shell_theme_dark()}</button>
          <button type="button" aria-pressed={theme === 'light'} onclick={() => onThemeChange('light')}><Sun size={14} aria-hidden="true" />{m.shell_theme_light()}</button>
        </div>
      </DropdownMenu.Group>

      <DropdownMenu.Separator class="um-sep" />

      <DropdownMenu.Item class="um-item danger" onSelect={onSignOut}>
        <LogOut size={14} aria-hidden="true" />
        <span class="um-label">{m.nav_sign_out()}</span>
      </DropdownMenu.Item>
    </DropdownMenu.Content>
  </DropdownMenu.Portal>
</DropdownMenu.Root>

<style>
  :global(.um-trigger) {
    display: flex; align-items: center; gap: var(--space-3);
    width: 100%; min-height: var(--control-h-lg);
    padding: var(--space-1) var(--space-2);
    color: var(--text-2);
    background: none;
    border: var(--border-w) solid transparent;
    border-radius: var(--radius);
    font: inherit; text-align: left; cursor: pointer;
  }
  :global(.um-trigger:hover), :global(.um-trigger[data-state='open']) {
    color: var(--text-1); background: var(--glass-bg-strong); border-color: var(--glass-border);
  }
  .um-avatar {
    display: grid; place-items: center; flex-shrink: 0;
    width: var(--control-h-sm); height: var(--control-h-sm);
    border-radius: var(--radius-full);
    background: var(--accent); color: var(--on-accent);
    font-size: var(--text-xs); font-weight: var(--weight-bold);
  }
  .um-who { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: var(--space-0); }
  .um-role { color: var(--text-1); font-size: var(--text-sm); font-weight: var(--weight-medium); text-transform: capitalize; }
  .um-lang { display: flex; align-items: center; gap: var(--space-1); color: var(--text-3); font-size: var(--text-xs); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .flag { width: var(--icon); height: auto; aspect-ratio: 7 / 5; border-radius: var(--radius-xs); box-shadow: 0 0 0 var(--border-w) var(--border); flex-shrink: 0; }

  :global(.um-content) {
    z-index: var(--z-dialog);
    min-width: calc(var(--sidebar-w) - 2 * var(--space-3));
    padding: var(--space-2);
    background: var(--glass-bg-strong);
    -webkit-backdrop-filter: var(--glass-blur);
    backdrop-filter: var(--glass-blur);
    border: var(--border-w) solid var(--glass-border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-2);
    animation: um-in var(--dur-2) var(--ease-out);
  }
  @keyframes um-in { from { opacity: 0; transform: translateY(var(--space-1)) scale(0.98); } }
  :global(.um-heading) { padding: var(--space-1) var(--space-2); color: var(--text-3); font-size: var(--text-xs); }
  :global(.um-item) {
    display: flex; align-items: center; gap: var(--space-2);
    min-height: var(--control-h);
    padding: 0 var(--space-2);
    color: var(--text-1); font-size: var(--text-sm);
    border-radius: var(--radius-sm);
    cursor: pointer; outline: none;
  }
  :global(.um-item[data-highlighted]) { background: var(--bg-hover); }
  :global(.um-item.danger) { color: var(--danger); }
  :global(.um-item.danger[data-highlighted]) { background: var(--danger-subtle); }
  .um-label { flex: 1; }
  :global(.um-sep) { height: var(--border-w); margin: var(--space-2) 0; background: var(--border); }
  .um-theme { display: flex; width: 100%; margin: var(--space-1) 0; }
  .um-theme > button { flex: 1; display: inline-flex; align-items: center; justify-content: center; gap: var(--space-1); }
</style>
