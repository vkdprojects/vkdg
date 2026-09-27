<script lang="ts">
  import '../app.css';
  import type { Snippet } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { m } from '$lib/paraglide/messages.js';
  import { getLocale, setLocale } from '$lib/paraglide/runtime.js';
  import { Toaster } from 'svelte-sonner';
  import { Home, Plug, Combine, Route, List, Gamepad2, Key, Settings, Puzzle, UserCircle } from 'lucide-svelte';
  import { Logo } from '$lib/components/index.js';
  import { api } from '$lib/api.js';
  import type { SessionUser, SystemInfo } from '$lib/api.js';

  interface Props {
    children: Snippet;
  }
  let { children }: Props = $props();

  let user = $state<SessionUser | null>(null);
  // Public pages render without the app shell: a signed-out visitor sees only
  // the sign-in form, not navigation to pages that would bounce them back.
  const isPublic = $derived(page.url.pathname === '/login' || page.url.pathname === '/logout');
  let system = $state<SystemInfo | null>(null);

  const navGroups = [
    {
      label: 'GATEWAY',
      items: [
        { href: '/', icon: Home, label: () => m.nav_overview() },
        { href: '/connections', icon: Plug, label: () => m.nav_connections() },
        { href: '/combos', icon: Combine, label: () => m.nav_combos() },
        { href: '/routes', icon: Route, label: () => m.nav_routes() },
      ],
    },
    {
      label: 'OBSERVE',
      items: [
        { href: '/requests', icon: List, label: () => m.nav_requests() },
        { href: '/playground', icon: Gamepad2, label: () => m.nav_playground() },
      ],
    },
    {
      label: 'MANAGE',
      items: [
        { href: '/keys', icon: Key, label: () => m.nav_keys() },
        { href: '/accounts', icon: UserCircle, label: () => m.nav_accounts() },
        { href: '/plugins', icon: Puzzle, label: () => 'Plugins' },
        { href: '/settings', icon: Settings, label: () => m.nav_settings() },
      ],
    },
  ];

  function isActive(href: string): boolean {
    const path = page.url.pathname;
    if (href === '/') return path === '/';
    return path === href || path.startsWith(href + '/');
  }

  async function signOut() {
    try { await api.logout(); } catch { /* ignore */ }
    goto('/login');
  }

  // Re-check the session on every navigation into the app: signing in lands on
  // `/` without a reload, so a one-time check on mount would never see it.
  $effect(() => {
    const path = page.url.pathname;
    if (isPublic) {
      user = null;
      return;
    }
    if (user) return;
    Promise.all([api.me(), api.system()])
      .then(([me, sys]) => {
        user = me;
        system = sys;
      })
      .catch(() => {
        if (page.url.pathname === path) goto('/login');
      });
  });
</script>

{#if isPublic || !user}
  <main class="main public">
    {#if isPublic}{@render children()}{/if}
  </main>
{:else}
<div class="shell">
  <aside class="sidebar">
    <a href="/" class="logo" aria-label="VKDG home">
      <Logo size={36} />
      <span class="logo-text">VKDG</span>
    </a>

    <nav class="nav">
      {#each navGroups as group}
        <div class="nav-group">
          <span class="nav-group-label">{group.label}</span>
          {#each group.items as item}
            <a
              href={item.href}
              class="nav-item"
              class:active={isActive(item.href)}
              aria-current={isActive(item.href) ? 'page' : undefined}
            >
              <item.icon size={15} strokeWidth={1.75} />
              {item.label()}
            </a>
          {/each}
        </div>
      {/each}
    </nav>

    <div class="sidebar-footer">
      <div class="locale-switcher">
        <button
          class:active={getLocale() === 'en'}
          aria-pressed={getLocale() === 'en'}
          onclick={() => setLocale('en')}
        >EN</button>
        <span class="sep">/</span>
        <button
          class:active={getLocale() === 'pt-BR'}
          aria-pressed={getLocale() === 'pt-BR'}
          onclick={() => setLocale('pt-BR')}
        >PT</button>
      </div>

      {#if user}
        <span class="user-role">{user.role}</span>
        <button type="button" class="signout" onclick={signOut}>{m.nav_sign_out()}</button>
      {/if}

      {#if system?.version}
        <span class="version">v{system.version}</span>
      {/if}
    </div>
  </aside>

  <main class="main">
    {@render children()}
  </main>
</div>
{/if}

<Toaster richColors theme="dark" position="bottom-right" />

<style>
  .shell {
    display: flex;
    height: 100vh;
    overflow: hidden;
  }

  .sidebar {
    width: 220px;
    min-width: 220px;
    background: var(--bg-surface);
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    overflow-y: auto;
  }

  .logo {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 20px 16px 16px;
    border-bottom: 1px solid var(--border);
    text-decoration: none;
  }

  .logo-text {
    font-size: 15px;
    font-weight: 700;
    letter-spacing: 0.06em;
    color: var(--text-1);
  }

  .nav {
    flex: 1;
    padding: 8px 0;
  }

  .nav-group {
    margin-bottom: 4px;
  }

  .nav-group-label {
    display: block;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.08em;
    color: var(--text-3);
    padding: 8px 16px 4px;
    text-transform: uppercase;
  }

  .nav-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 16px;
    color: var(--text-2);
    text-decoration: none;
    font-size: 13px;
    transition: background 0.1s, color 0.1s;
    border-right: 2px solid transparent;
  }

  .nav-item:hover {
    background: var(--bg-hover);
    color: var(--text-1);
  }

  .nav-item.active {
    background: color-mix(in oklch, var(--accent) 10%, transparent);
    color: var(--text-1);
    border-right-color: var(--accent);
  }

  .sidebar-footer {
    padding: 12px 16px;
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 8px;
    font-size: 12px;
  }

  .locale-switcher {
    display: flex;
    align-items: center;
    gap: 4px;
    color: var(--text-3);
  }

  .locale-switcher button {
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-3);
    font-size: 11px;
    font-weight: 500;
    padding: 2px 4px;
    border-radius: var(--radius-sm);
    transition: color 0.1s;
  }

  .locale-switcher button:hover {
    color: var(--text-2);
  }

  .locale-switcher button.active {
    color: var(--accent);
  }

  .sep {
    color: var(--text-3);
    font-size: 10px;
  }

  .user-role {
    color: var(--text-3);
    font-size: 11px;
    text-transform: capitalize;
  }

  .signout {
    background: none;
    border: none;
    color: var(--text-2);
    cursor: pointer;
    font-size: 12px;
    padding: 0;
    text-align: left;
    transition: color 0.1s;
  }

  .signout:hover {
    color: var(--danger);
  }

  .version {
    color: var(--text-3);
    font-size: 11px;
  }

  .main {
    flex: 1;
    overflow-y: auto;
    background: var(--bg-base);
  }
</style>
