<script lang="ts">
  import '../app.css';
  import type { Snippet } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { m } from '$lib/paraglide/messages.js';
    import { Toaster } from 'svelte-sonner';
  import { Home, Combine, Route, List, Gamepad2, Key, Settings, Puzzle, LayoutGrid, Cable, Users, Gauge, PanelLeft, Menu } from 'lucide-svelte';
  import { Logo, StatusDot, UserMenu } from '$lib/components/index.js';
  import { api } from '$lib/api.js';
  import { poll } from '$lib/live.svelte.js';
  import type { SessionUser, SystemInfo } from '$lib/api.js';

  interface Props { children: Snippet; }
  let { children }: Props = $props();

  let user = $state<SessionUser | null>(null);
  let system = $state<SystemInfo | null>(null);
  const isPublic = $derived(page.url.pathname === '/login' || page.url.pathname === '/logout');

  // Collapsed = icon rail only. Persisted so it doesn't reset on navigation;
  // read eagerly (not in onMount) so the very first render already matches
  // the stored preference and the sidebar doesn't visibly snap on load.
  const COLLAPSE_KEY = 'vkdg.nav.collapsed';
  let collapsed = $state(typeof localStorage !== 'undefined' && localStorage.getItem(COLLAPSE_KEY) === '1');
  // Hover/focus while collapsed pops the rail open as an overlay without
  // pushing content or touching the persisted preference.
  let peeking = $state(false);

  function toggleCollapsed() {
    collapsed = !collapsed;
    localStorage.setItem(COLLAPSE_KEY, collapsed ? '1' : '0');
    // The toggle button lives inside the sidebar's own mouseenter/mouseleave
    // region, so clicking it never fires a mouseleave. Without this, a
    // click made while peek-expanded (mouse still over the rail) leaves
    // `peeking` stuck true: with `collapsed` now flipped, the two states
    // become visually indistinguishable (both render at full width), so it
    // reads as "stuck expanded" even though the class list is technically
    // consistent. Forcing peeking false makes the post-click state always
    // exactly match `collapsed` until the next real hover/focus.
    peeking = false;
  }

  let theme = $state<'dark' | 'light'>(
    typeof document !== 'undefined' && document.documentElement.dataset.theme === 'light' ? 'light' : 'dark',
  );
  function setTheme(next: 'dark' | 'light') {
    theme = next;
    document.documentElement.dataset.theme = theme;
    localStorage.setItem('vkdg.theme', theme);
  }

  // Mobile: off-canvas drawer, closed on every navigation.
  let drawerOpen = $state(false);
  $effect(() => { page.url.pathname; drawerOpen = false; });

  const navGroups = [
    {
      label: () => m.nav_group_gateway(),
      items: [
        { href: '/', icon: Home, label: () => m.nav_overview() },
        { href: '/connections', icon: Cable, label: () => m.nav_connections() },
        { href: '/combos', icon: Combine, label: () => m.nav_combos() },
        { href: '/routes', icon: Route, label: () => m.nav_routes() },
        { href: '/strategies', icon: Gauge, label: () => m.nav_strategies() },
      ],
    },
    {
      label: () => m.nav_group_observe(),
      items: [
        { href: '/requests', icon: List, label: () => m.nav_requests() },
        { href: '/playground', icon: Gamepad2, label: () => m.nav_playground() },
      ],
    },
    {
      label: () => m.nav_group_manage(),
      items: [
        { href: '/providers', icon: LayoutGrid, label: () => m.nav_providers() },
        { href: '/accounts', icon: Users, label: () => m.nav_accounts() },
        { href: '/keys', icon: Key, label: () => m.nav_keys() },
        { href: '/plugins', icon: Puzzle, label: () => m.nav_plugins() },
        { href: '/settings', icon: Settings, label: () => m.nav_settings() },
      ],
    },
  ];

  function isActive(href: string): boolean {
    const path = page.url.pathname;
    if (href === '/') return path === '/';
    return path === href || path.startsWith(href + '/');
  }

  function formatUptime(secs: number): string {
    const days = Math.floor(secs / 86400);
    const hours = Math.floor((secs % 86400) / 3600);
    return days > 0 ? `${days}d ${hours}h` : `${hours}h ${Math.floor((secs % 3600) / 60)}m`;
  }

  async function signOut() {
    try { await api.logout(); } catch { /* ignore */ }
    goto('/login');
  }

  $effect(() => {
    const path = page.url.pathname;
    if (isPublic) { user = null; system = null; return; }
    if (user) return;
    Promise.all([api.me(), api.system()])
      .then(([me, sys]) => { user = me; system = sys; })
      .catch(() => { if (page.url.pathname === path) goto('/login'); });
  });

  poll(async () => {
    if (isPublic || !user) return;
    try { system = await api.system(); } catch { /* retain last known state */ }
  });
</script>

{#if isPublic || !user}
  <main class="main public">{#if isPublic}{@render children()}{/if}</main>
{:else}
  <div class="shell" class:collapsed class:drawer-open={drawerOpen}>
    <button type="button" class="scrim" aria-label={m.shell_collapse_nav()} onclick={() => (drawerOpen = false)}></button>
    <div class="sidebar-slot" class:peeking>
      <nav
        class="sidebar"
        aria-label={m.shell_primary_navigation()}
        onmouseenter={() => (peeking = true)}
        onmouseleave={() => (peeking = false)}
        onfocusin={() => (peeking = true)}
        onfocusout={(e) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node | null)) peeking = false;
        }}
      >
        <a href="/" class="brand" aria-label="VKDG home">
          <span class="brand-mark"><Logo size={22} /></span><span class="brand-name">VKDG</span>
        </a>

        <button
          type="button"
          class="collapse-toggle"
          onclick={toggleCollapsed}
          aria-label={collapsed ? m.shell_expand_nav() : m.shell_collapse_nav()}
          aria-pressed={collapsed}
        >
          <PanelLeft size={14} strokeWidth={1.8} aria-hidden="true" />
        </button>

        <div class="nav-groups">
          {#each navGroups as group}
            <div class="nav-group">
              <span class="nav-group-label">{group.label()}</span>
              <div class="nav-items">
                {#each group.items as item}
                  <a href={item.href} class="nav-item" class:active={isActive(item.href)} aria-current={isActive(item.href) ? 'page' : undefined} title={item.label()}>
                    <item.icon size={17} strokeWidth={1.75} aria-hidden="true" /><span class="nav-item-label">{item.label()}</span>
                  </a>
                {/each}
              </div>
            </div>
          {/each}
        </div>

        <div class="sidebar-foot">
          <UserMenu
            role={user.role}
            {theme}
            onThemeChange={setTheme}
            onSignOut={signOut}
            compact={collapsed && !peeking && !drawerOpen}
          />
        </div>
      </nav>
    </div>

    <div class="content">
      <header class="topbar">
        <button type="button" class="icon-btn menu-btn" onclick={() => (drawerOpen = true)} aria-label={m.shell_expand_nav()}>
          <Menu size={18} aria-hidden="true" />
        </button>
        {#if system}
          <div class="vitals" aria-label={m.shell_gateway_telemetry()}>
            <span class="health" data-status={system.status}><StatusDot status={system.status} /><span>{system.status}</span></span>
            <span class="vital"><span class="k">{m.system_active_requests()}</span><strong class="mono">{system.active_requests}</strong></span>
            <span class="vital"><span class="k">{m.system_uptime()}</span><strong class="mono">{formatUptime(system.uptime_secs)}</strong></span>
            <span class="vital hide-sm"><span class="k">{m.system_version()}</span><strong class="mono">v{system.version}</strong></span>
          </div>
        {/if}
      </header>
      <main class="main">{@render children()}</main>
    </div>
  </div>
{/if}

<Toaster richColors {theme} position="bottom-right" />

<style>
  .shell {
    min-height: 100vh;
    display: grid;
    grid-template-columns: var(--sidebar-w) minmax(0, 1fr);
    transition: grid-template-columns var(--dur-2) var(--ease-out);
  }
  .shell.collapsed { grid-template-columns: var(--sidebar-w-collapsed) minmax(0, 1fr); }

  .sidebar-slot { position: sticky; top: 0; height: 100vh; z-index: var(--z-nav); width: var(--sidebar-w); }
  .shell.collapsed .sidebar-slot { width: var(--sidebar-w-collapsed); }

  .sidebar {
    position: relative;
    display: flex; flex-direction: column;
    background: var(--glass-bg);
    -webkit-backdrop-filter: var(--glass-blur);
    backdrop-filter: var(--glass-blur);
    border-right: var(--border-w) solid var(--glass-border);
    box-shadow: var(--glass-highlight);
    width: 100%; height: 100%;
  }
  .sidebar-slot.peeking .sidebar {
    position: absolute; inset: 0 auto 0 0;
    width: var(--sidebar-w);
    box-shadow: var(--shadow-2);
  }

  .brand {
    display: flex; align-items: center; gap: var(--space-3); min-width: 0;
    padding: var(--space-4) var(--space-4) var(--space-3);
    color: var(--text-1);
    font-size: var(--text-md); font-weight: var(--weight-bold); letter-spacing: var(--tracking-tight);
  }
  .brand-mark {
    display: grid; place-items: center; flex-shrink: 0;
    width: var(--control-h-sm); height: var(--control-h-sm); border-radius: var(--radius);
    background: var(--accent);
    color: var(--on-accent);
    box-shadow: var(--glass-highlight);
  }
  .brand-name { overflow: hidden; white-space: nowrap; font-family: var(--font-dot); font-size: var(--text-lg); letter-spacing: var(--tracking-dot); }
  .shell.collapsed .sidebar-slot:not(.peeking) .brand-name { display: none; }

  .collapse-toggle {
    position: absolute; top: var(--space-5); right: calc(var(--space-3) * -1);
    display: grid; place-items: center;
    width: var(--space-5); height: var(--space-5);
    background: var(--bg-elevated);
    border: var(--border-w) solid var(--border-strong);
    border-radius: var(--radius-full);
    color: var(--text-2);
    cursor: pointer; z-index: var(--z-raised);
    opacity: 0;
  }
  .sidebar:hover .collapse-toggle, .collapse-toggle:focus-visible { opacity: 1; }
  .collapse-toggle:hover { color: var(--text-1); border-color: var(--accent); }
  .shell.collapsed .collapse-toggle { transform: rotate(180deg); }

  .nav-groups { flex: 1; overflow-y: auto; overflow-x: hidden; padding: var(--space-1) var(--space-3); }
  .nav-group + .nav-group { margin-top: var(--space-3); }
  .nav-group-label { display: block; color: var(--text-3); font-size: var(--text-2xs); font-weight: var(--weight-medium); padding: var(--space-1) var(--space-3) var(--space-2); white-space: nowrap; }
  .shell.collapsed .sidebar-slot:not(.peeking) .nav-group-label { visibility: hidden; height: var(--space-2); padding: 0; }
  .nav-items { display: flex; flex-direction: column; gap: var(--space-0); }
  .nav-item {
    position: relative;
    display: flex; align-items: center; gap: var(--space-3);
    color: var(--text-2);
    font-size: var(--text-sm); font-weight: var(--weight-medium);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius);
    white-space: nowrap; overflow: hidden;
  }
  .nav-item :global(svg) { flex-shrink: 0; transition: transform var(--dur-2) var(--ease-spring); }
  .nav-item:hover { color: var(--text-1); background: var(--bg-hover); }
  .nav-item:hover :global(svg) { transform: scale(1.08); }
  .nav-item.active { color: var(--text-1); background: var(--glass-bg-strong); box-shadow: var(--glass-highlight); }
  .nav-item.active :global(svg) { color: var(--accent); }
  .nav-item.active::before {
    content: ''; position: absolute; left: 0; top: 22%; bottom: 22%; width: var(--indicator-w);
    border-radius: 0 var(--radius-xs) var(--radius-xs) 0; background: var(--accent);
  }
  .shell.collapsed .sidebar-slot:not(.peeking) .nav-item { justify-content: center; }
  .shell.collapsed .sidebar-slot:not(.peeking) .nav-item-label { display: none; }

  .sidebar-foot { padding: var(--space-2); border-top: var(--border-w) solid var(--border); }

  .content { min-width: 0; display: flex; flex-direction: column; container-type: inline-size; container-name: content; }

  .topbar {
    position: sticky; top: 0; z-index: var(--z-topbar);
    display: flex; align-items: center; gap: var(--space-3);
    min-height: var(--topbar-h); padding: 0 var(--page-gutter);
    background: color-mix(in oklch, var(--bg-base) 55%, transparent);
    -webkit-backdrop-filter: var(--glass-blur);
    backdrop-filter: var(--glass-blur);
    border-bottom: var(--border-w) solid var(--border);
  }
  .menu-btn { display: none; }
  .vitals { display: flex; align-items: center; gap: var(--space-2); margin-left: auto; overflow-x: auto; scrollbar-width: none; }
  .vitals > span {
    display: inline-flex; align-items: center; gap: var(--space-2); white-space: nowrap;
    padding: var(--space-1) var(--space-3); border-radius: var(--radius-full);
    background: var(--glass-bg); border: var(--border-w) solid var(--glass-border); box-shadow: var(--glass-highlight);
    font-size: var(--text-xs);
  }
  .vitals .k { color: var(--text-3); }
  .vitals strong { color: var(--text-1); font-weight: var(--weight-medium); }
  .health { color: var(--text-1); text-transform: capitalize; }
  .health[data-status='healthy'] { border-color: color-mix(in oklch, var(--success) 35%, transparent); background: var(--success-subtle); }
  .health[data-status='degraded'] { border-color: color-mix(in oklch, var(--warning) 35%, transparent); background: var(--warning-subtle); }

  .main { min-width: 0; flex: 1; }
  .scrim { display: none; }

  @media (max-width: 800px) {
    .shell, .shell.collapsed { grid-template-columns: minmax(0, 1fr); }
    .sidebar-slot, .shell.collapsed .sidebar-slot {
      position: fixed; inset: 0 auto 0 0; width: min(var(--sidebar-w), 86vw);
      transform: translateX(-100%);
      transition: transform var(--dur-2) var(--ease-out);
    }
    .shell.drawer-open .sidebar-slot { transform: none; box-shadow: var(--shadow-2); }
    .sidebar-slot.peeking .sidebar { position: relative; width: 100%; box-shadow: none; }
    /* Drawer always shows full labels regardless of the desktop collapse pref. */
    .shell.collapsed .sidebar-slot .brand-name,
    .shell.collapsed .sidebar-slot .nav-item-label { display: revert; }
    .shell.collapsed .sidebar-slot .nav-item { justify-content: flex-start; }
    .shell.collapsed .sidebar-slot .nav-group-label { visibility: visible; height: auto; padding: var(--space-1) var(--space-3) var(--space-2); }
    .collapse-toggle { display: none; }
    .scrim {
      display: block; position: fixed; inset: 0; z-index: var(--z-scrim); border: 0;
      background: var(--bg-scrim);
      opacity: 0; pointer-events: none; transition: opacity var(--dur-2);
    }
    .shell.drawer-open .scrim { opacity: 1; pointer-events: auto; }
    .menu-btn { display: grid; }
    .hide-sm, .vitals .k { display: none; }
  }
</style>
