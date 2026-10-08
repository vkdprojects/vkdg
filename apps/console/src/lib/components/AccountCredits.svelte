<script lang="ts">
  import type { Account } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { formatNumber, formatDate } from '$lib/format.js';
  import Meter from './Meter.svelte';
  import Badge from './Badge.svelte';

  /**
   * Renders an account's plan credits honestly: a real meter when the
   * upstream reported usage/limit, an explicit "unavailable" state when it
   * didn't, and nothing at all when the provider has no credits concept.
   * Shared by the provider detail, overview and accounts pages so the three
   * screens agree on what "credits" means.
   */
  interface Props {
    account: Account;
    /** `compact` drops the panel chrome/foot for use inside a dense table cell. */
    variant?: 'panel' | 'compact';
  }

  let { account, variant = 'panel' }: Props = $props();

  /** Coarse "checked X ago" — a cache-age hint, not a live clock. */
  function checkedAgo(iso: string): string {
    const seconds = Math.max(0, Math.round((Date.now() - new Date(iso).getTime()) / 1000));
    if (seconds < 60) return m.credits_checked_seconds({ n: seconds });
    const minutes = Math.round(seconds / 60);
    if (minutes < 60) return m.credits_checked_minutes({ n: minutes });
    const hours = Math.round(minutes / 60);
    if (hours < 24) return m.credits_checked_hours({ n: hours });
    return m.credits_checked_days({ n: Math.round(hours / 24) });
  }

  const reported = $derived(account.credits_source === 'reported' && account.credits_used != null && account.credits_limit != null);
  const unavailable = $derived(account.credits_source === 'unavailable');
</script>

{#if reported}
  <div class="limits" data-state="reported" data-variant={variant}>
    {#if variant === 'panel'}
      <div class="limits-head"><span>{m.plan_limits_title()}</span>{#if account.credits_plan}<span class="plan-name">{account.credits_plan}</span>{/if}</div>
    {/if}
    <Meter
      value={account.credits_used ?? 0}
      limit={account.credits_limit}
      valueText={`${formatNumber(account.credits_used ?? 0)} / ${formatNumber(account.credits_limit ?? 0)}`}
    />
    {#if variant === 'panel'}
      <div class="limits-foot">
        {#if account.credits_period_end != null}<span>{m.credits_resets_on({ date: formatDate(account.credits_period_end * 1000) })}</span>{/if}
        {#if account.credits_checked_at}<span class="checked-at">{m.credits_last_checked({ time: checkedAgo(account.credits_checked_at) })}</span>{/if}
      </div>
    {:else if account.credits_plan}
      <span class="compact-plan mono">{account.credits_plan}</span>
    {/if}
  </div>
{:else if unavailable}
  <div class="limits" data-state="unavailable" data-variant={variant}>
    {#if variant === 'panel'}
      <div class="limits-head"><span>{m.plan_limits_title()}</span><Badge status="unknown" label={m.plan_limits_unavailable()} /></div>
      <p>{m.plan_limits_unavailable_desc()}</p>
    {:else}
      <Badge status="unknown" label={m.plan_limits_unavailable()} />
    {/if}
  </div>
{:else if variant === 'compact'}
  <span class="no-credits">—</span>
{/if}

<style>
  .limits[data-variant='panel'] { padding: 11px 16px; background: var(--bg-inset); border-bottom: 1px solid var(--border); display: flex; flex-direction: column; gap: 8px; }
  .limits[data-variant='compact'] { display: flex; flex-direction: column; gap: 4px; min-width: 11rem; }
  .limits-head { display: flex; align-items: center; gap: 8px; color: var(--text-1); font-size: var(--text-sm); font-weight: 600; }
  .plan-name { color: var(--text-3); font-size: var(--text-xs); font-weight: 500; text-transform: none; }
  .limits p { margin: 0; font-size: var(--text-xs); color: var(--text-2); }
  .limits-foot { display: flex; justify-content: space-between; gap: 8px; font-size: var(--text-2xs); color: var(--text-3); }
  .limits-foot .checked-at { color: var(--text-3); }
  .compact-plan { color: var(--text-3); font-size: var(--text-2xs); }
  .no-credits { color: var(--text-3); }
</style>
