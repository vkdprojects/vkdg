<script lang="ts">
  import '../app.css';
  import type { Snippet } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { m } from '$lib/paraglide/messages.js';
  import { getLocale, setLocale } from '$lib/paraglide/runtime.js';
  import { Toaster } from 'svelte-sonner';
  import { Home, Combine, Route, List, Gamepad2, Key, Settings, Puzzle, LayoutGrid, Cable, Users, Gauge } from 'lucide-svelte';
  import { Logo, StatusDot } from '$lib/components/index.js';
  import { api } from '$lib/api.js';
  import type { SessionUser, SystemInfo } from '$lib/api.js';

  interface Props { children: Snippet; }
  let { children }: Props = $props();

  let user = $state<SessionUser | null>(null);
  let system = $state<SystemInfo | null>(null);
  const isPublic = $derived(page.url.pathname === '/login' || page.url.pathname === '/logout');

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
  <div class="shell">
    <header class="topbar">
      <a href="/" class="brand" aria-label="VKDG home">
        <Logo size={25} /><span>VKDG</span><span class="brand-tag">console</span>
      </a>
      {#if system}
        <div class="telemetry" aria-label={m.shell_gateway_telemetry()}>
          <div class="health"><StatusDot status={system.status} /><span>{system.status}</span></div>
          <div><span>{m.system_version()}</span><strong class="mono">v{system.version}</strong></div>
          <div><span>{m.system_uptime()}</span><strong class="mono">{formatUptime(system.uptime_secs)}</strong></div>
          <div><span>{m.system_active_requests()}</span><strong class="mono">{system.active_requests}</strong></div>
        </div>
      {/if}
      <div class="session">
        <div class="locale-switcher" aria-label={m.shell_language()}>
          <button class:active={getLocale() === 'en'} aria-pressed={getLocale() === 'en'} onclick={() => setLocale('en')}>EN</button>
          <button class:active={getLocale() === 'pt-BR'} aria-pressed={getLocale() === 'pt-BR'} onclick={() => setLocale('pt-BR')}>PT</button>
        </div>
        <span class="role mono">{user.role}</span>
        <button type="button" class="signout" onclick={signOut}>{m.nav_sign_out()}</button>
      </div>
    </header>

    <nav class="nav" aria-label={m.shell_primary_navigation()}>
      {#each navGroups as group}
        <div class="nav-group">
          <span class="nav-group-label">{group.label()}</span>
          <div class="nav-items">
            {#each group.items as item}
              <a href={item.href} class="nav-item" class:active={isActive(item.href)} aria-current={isActive(item.href) ? 'page' : undefined}>
                <item.icon size={14} strokeWidth={1.8} aria-hidden="true" />{item.label()}
              </a>
            {/each}
          </div>
        </div>
      {/each}
    </nav>
    <main class="main">{@render children()}</main>
  </div>
{/if}

<Toaster richColors theme="dark" position="bottom-right" />

<style>
  .shell { min-height: 100vh; display: grid; grid-template-rows: auto auto 1fr; }
  .topbar { min-height: 56px; display: grid; grid-template-columns: auto minmax(0, 1fr) auto; align-items: stretch; background: var(--bg-surface); border-bottom: 1px solid var(--border); }
  .brand { display: flex; align-items: center; gap: 8px; min-width: 180px; padding: 0 20px; color: var(--text-1); text-decoration: none; font-size: 14px; font-weight: 700; letter-spacing: .06em; border-right: 1px solid var(--border); }
  .brand-tag { color: var(--text-3); font-size: 10px; font-weight: 500; letter-spacing: .04em; }
  .telemetry { display: flex; align-items: stretch; min-width: 0; }
  .telemetry > div { display: flex; align-items: center; gap: 7px; padding: 0 16px; border-right: 1px solid var(--border); white-space: nowrap; }
  .telemetry span { color: var(--text-3); font-size: var(--text-2xs); text-transform: uppercase; letter-spacing: .05em; }
  .telemetry strong { color: var(--text-1); font-size: var(--text-xs); font-weight: 500; }
  .telemetry .health span { color: var(--text-1); }
  .session { display: flex; align-items: center; gap: 12px; padding: 0 18px; }
  .locale-switcher { display: flex; border: 1px solid var(--border); border-radius: var(--radius-sm); }
  .locale-switcher button, .signout { border: 0; background: none; color: var(--text-3); cursor: pointer; font-size: var(--text-2xs); padding: 4px 6px; }
  .locale-switcher button.active { background: var(--accent-subtle); color: var(--accent); }
  .role { color: var(--text-3); font-size: var(--text-2xs); text-transform: uppercase; }
  .signout:hover { color: var(--danger); }
  .nav { display: flex; align-items: stretch; gap: 0; background: var(--bg-inset); border-bottom: 1px solid var(--border); padding: 0 12px; }
  .nav-group { display: flex; align-items: center; min-width: max-content; border-right: 1px solid var(--border); padding: 4px 9px; }
  .nav-group:last-child { border-right: 0; }
  .nav-group-label { color: var(--text-3); font-size: 9px; font-weight: 600; letter-spacing: .08em; text-transform: uppercase; padding: 0 6px; }
  .nav-items { display: flex; align-items: center; gap: 2px; }
  .nav-item { display: flex; align-items: center; gap: 5px; color: var(--text-2); text-decoration: none; font-size: var(--text-xs); padding: 6px 8px; border-radius: var(--radius-sm); white-space: nowrap; }
  .nav-item:hover { color: var(--text-1); background: var(--bg-hover); }
  .nav-item.active { color: var(--text-1); background: var(--accent-subtle); box-shadow: inset 0 -2px var(--accent); }
  .main { min-width: 0; background: var(--bg-base); }
  @media (max-width: 1050px) {
    .nav { flex-wrap: wrap; padding: 3px 8px; }
    .nav-group { border-right: 0; padding: 2px 5px; }
    .nav-group-label { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden; clip: rect(0,0,0,0); white-space: nowrap; border: 0; }
    .telemetry > div { padding: 0 10px; }
    .telemetry > div span { display: none; }
  }
  @media (max-width: 700px) {
    .topbar { grid-template-columns: auto 1fr; }
    .brand { min-width: 0; padding: 0 12px; border-right: 0; }
    .brand-tag, .role { display: none; }
    .telemetry { grid-column: 1 / -1; grid-row: 2; border-top: 1px solid var(--border); justify-content: space-around; }
    .telemetry > div { flex: 1; justify-content: center; padding: 7px 4px; }
    .session { justify-self: end; padding: 8px 12px; }
    .nav { display: grid; grid-template-columns: 1fr; padding: 5px 8px; }
    .nav-group { min-width: 0; align-items: flex-start; border-bottom: 1px solid var(--border); padding: 4px 0; }
    .nav-group:last-child { border-bottom: 0; }
    .nav-items { flex-wrap: wrap; }
    .nav-item { padding: 7px 8px; }
  }
</style>
