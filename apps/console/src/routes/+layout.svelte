<script lang="ts">
  import '../app.css';
  import type { Snippet } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { m } from '$lib/paraglide/messages.js';
  import { getLocale, setLocale } from '$lib/paraglide/runtime.js';
  import { Toaster } from 'svelte-sonner';
  import { Home, Combine, Route, List, Gamepad2, Key, Settings, Puzzle, LayoutGrid, Cable, Users, Gauge, PanelLeft } from 'lucide-svelte';
  import { Logo, StatusDot } from '$lib/components/index.js';
  import { api } from '$lib/api.js';
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

  $effect(() => {
    if (isPublic || !user) return;
    let timer: ReturnType<typeof setInterval> | undefined;
    const refresh = async () => {
      if (document.visibilityState !== 'visible') return;
      try { system = await api.system(); } catch { /* retain last known state */ }
    };
    const sync = () => {
      clearInterval(timer);
      timer = undefined;
      if (document.visibilityState === 'visible') timer = setInterval(refresh, 5000);
    };
    sync();
    document.addEventListener('visibilitychange', sync);
    return () => { clearInterval(timer); document.removeEventListener('visibilitychange', sync); };
  });
</script>

{#if isPublic || !user}
  <main class="main public">{#if isPublic}{@render children()}{/if}</main>
{:else}
  <div class="shell" class:collapsed class:peeking>
    <nav
      class="sidebar"
      aria-label={m.shell_primary_navigation()}
      onmouseenter={() => (peeking = true)}
      onmouseleave={() => (peeking = false)}
      onfocusin={() => (peeking = true)}
      onfocusout={(e) => {
        // Only collapse back when focus actually leaves the sidebar, not
        // when it moves between two links inside it.
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) peeking = false;
      }}
    >
      <a href="/" class="brand" aria-label="VKDG home">
        <Logo size={25} /><span class="brand-name">VKDG</span>
      </a>

      <button
        type="button"
        class="collapse-toggle"
        onclick={toggleCollapsed}
        aria-label={collapsed ? m.shell_expand_nav() : m.shell_collapse_nav()}
        aria-pressed={collapsed}
      >
        <PanelLeft size={15} strokeWidth={1.8} aria-hidden="true" />
      </button>

      <div class="nav-groups">
        {#each navGroups as group}
          <div class="nav-group">
            <span class="nav-group-label">{group.label()}</span>
            <div class="nav-items">
              {#each group.items as item}
                <a href={item.href} class="nav-item" class:active={isActive(item.href)} aria-current={isActive(item.href) ? 'page' : undefined}>
                  <item.icon size={16} strokeWidth={1.8} aria-hidden="true" /><span class="nav-item-label">{item.label()}</span>
                </a>
              {/each}
            </div>
          </div>
        {/each}
      </div>

      <div class="sidebar-foot">
        <div class="locale-switcher" aria-label={m.shell_language()}>
          <button class:active={getLocale() === 'en'} aria-pressed={getLocale() === 'en'} onclick={() => setLocale('en')}>EN</button>
          <button class:active={getLocale() === 'pt-BR'} aria-pressed={getLocale() === 'pt-BR'} onclick={() => setLocale('pt-BR')}>PT</button>
        </div>
        <span class="role mono">{user.role}</span>
        <button type="button" class="signout" onclick={signOut}>{m.nav_sign_out()}</button>
      </div>
    </nav>

    <div class="content">
      {#if system}
        <div class="status-bar" aria-label={m.shell_gateway_telemetry()}>
          <div class="health"><StatusDot status={system.status} /><span>{system.status}</span></div>
          <div><span>{m.system_version()}</span><strong class="mono">v{system.version}</strong></div>
          <div><span>{m.system_uptime()}</span><strong class="mono">{formatUptime(system.uptime_secs)}</strong></div>
          <div><span>{m.system_active_requests()}</span><strong class="mono">{system.active_requests}</strong></div>
        </div>
      {/if}
      <main class="main">{@render children()}</main>
    </div>
  </div>
{/if}

<Toaster richColors theme="dark" position="bottom-right" />

<style>
  /*
   * Sidebar shell. Two widths (expanded/collapsed) drive the grid column;
   * collapsing is a CSS variable flip, not a remount, so focus/scroll state
   * in the nav survives toggling. Collapsed width fits an icon + padding at
   * the current root font-size (see app.css clamp) without being cropped on
   * a 2K root size, since it's in rem, not a fixed px value.
   */
  .shell {
    --sidebar-w: 15.5rem;
    --sidebar-w-collapsed: 3.5rem;
    min-height: 100vh;
    display: grid;
    grid-template-columns: var(--sidebar-w) minmax(0, 1fr);
  }
  .shell.collapsed { grid-template-columns: var(--sidebar-w-collapsed) minmax(0, 1fr); }

  .sidebar {
    position: relative;
    z-index: 20;
    display: flex;
    flex-direction: column;
    background: var(--bg-surface);
    border-right: 1px solid var(--border);
    width: var(--sidebar-w);
    transition: width 0.15s ease;
  }
  .shell.collapsed .sidebar { width: var(--sidebar-w-collapsed); }
  /* Collapsed + hovered/focused: overlay back to full width without
     reflowing the grid column (content doesn't shift under it). */
  .shell.collapsed.peeking .sidebar {
    position: absolute;
    inset: 0 auto 0 0;
    width: var(--sidebar-w);
    box-shadow: 4px 0 24px rgba(0, 0, 0, 0.35);
  }

  .brand { display: flex; align-items: center; gap: 8px; min-width: 0; padding: 14px 16px; color: var(--text-1); text-decoration: none; font-size: var(--text-base); font-weight: 700; letter-spacing: .06em; border-bottom: 1px solid var(--border); }
  .brand-name { overflow: hidden; white-space: nowrap; }
  .shell.collapsed:not(.peeking) .brand-name { display: none; }

  .collapse-toggle {
    position: absolute;
    top: 12px;
    right: -13px;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 50%;
    color: var(--text-2);
    cursor: pointer;
    z-index: 1;
  }
  .collapse-toggle:hover { color: var(--text-1); border-color: var(--border-strong); }
  .shell.collapsed .collapse-toggle { transform: rotate(180deg); }

  .nav-groups { flex: 1; overflow-y: auto; padding: 10px 0; }
  .nav-group { padding: 6px 12px; }
  .nav-group-label { display: block; color: var(--text-3); font-size: 9px; font-weight: 600; letter-spacing: .08em; text-transform: uppercase; padding: 4px 10px; white-space: nowrap; }
  .shell.collapsed:not(.peeking) .nav-group-label { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden; clip: rect(0,0,0,0); }
  .nav-items { display: flex; flex-direction: column; gap: 1px; }
  .nav-item { display: flex; align-items: center; gap: 10px; color: var(--text-2); text-decoration: none; font-size: var(--text-sm); padding: 7px 10px; border-radius: var(--radius-sm); white-space: nowrap; overflow: hidden; }
  .nav-item :global(svg) { flex-shrink: 0; }
  .nav-item:hover { color: var(--text-1); background: var(--bg-hover); }
  .nav-item.active { color: var(--text-1); background: var(--accent-subtle); box-shadow: inset 2px 0 var(--accent); }
  .shell.collapsed:not(.peeking) .nav-item-label { display: none; }

  .sidebar-foot { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; padding: 12px; border-top: 1px solid var(--border); }
  .shell.collapsed:not(.peeking) .sidebar-foot { flex-direction: column; }
  .locale-switcher { display: flex; border: 1px solid var(--border); border-radius: var(--radius-sm); }
  .locale-switcher button, .signout { border: 0; background: none; color: var(--text-3); cursor: pointer; font-size: var(--text-2xs); padding: 4px 6px; }
  .locale-switcher button.active { background: var(--accent-subtle); color: var(--accent); }
  .role { color: var(--text-3); font-size: var(--text-2xs); text-transform: uppercase; }
  .shell.collapsed:not(.peeking) .role { display: none; }
  .signout:hover { color: var(--danger); }

  .content { min-width: 0; display: flex; flex-direction: column; background: var(--bg-base); container-type: inline-size; container-name: content; }

  /* Gateway vital signs, previously in the topbar: a thin strip above the
     page content keeps them visible at all times, including when the
     sidebar is collapsed to icons (where there's no room to show them). */
  .status-bar { display: flex; align-items: stretch; background: var(--bg-inset); border-bottom: 1px solid var(--border); overflow-x: auto; }
  .status-bar > div { display: flex; align-items: center; gap: 7px; padding: 6px 16px; border-right: 1px solid var(--border); white-space: nowrap; }
  .status-bar span { color: var(--text-3); font-size: var(--text-2xs); text-transform: uppercase; letter-spacing: .05em; }
  .status-bar strong { color: var(--text-1); font-size: var(--text-xs); font-weight: 500; }
  .status-bar .health span { color: var(--text-1); }

  .main { min-width: 0; flex: 1; }

  @media (max-width: 800px) {
    /* Icon rail by default on narrow screens; hover/focus peek still works,
       and the explicit toggle still expands it in place if preferred. */
    .shell:not(.collapsed) { grid-template-columns: var(--sidebar-w-collapsed) minmax(0, 1fr); }
    .shell:not(.collapsed) .sidebar { width: var(--sidebar-w-collapsed); }
    .shell:not(.collapsed):not(.peeking) .brand-name,
    .shell:not(.collapsed):not(.peeking) .nav-group-label,
    .shell:not(.collapsed):not(.peeking) .nav-item-label,
    .shell:not(.collapsed):not(.peeking) .role { display: none; }
    .shell:not(.collapsed).peeking .sidebar {
      position: absolute;
      inset: 0 auto 0 0;
      width: var(--sidebar-w);
      box-shadow: 4px 0 24px rgba(0, 0, 0, 0.35);
    }
    .status-bar > div span { display: none; }
  }
</style>
