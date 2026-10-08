<script lang="ts">
  import type { ClientKey } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { Badge, Button, Meter } from '$lib/components/index.js';
  import { formatNumber, formatDateTime } from '$lib/format.js';
  import { humanizeDate, keyStatusLabels } from './keyLimits.js';

  interface Props {
    k: ClientKey;
    expanded: boolean;
    busy: boolean;
    ontoggle: () => void;
    onedit: () => void;
    onregenerate: () => void;
    ontoggledisabled: () => void;
    onrevoke: () => void;
  }

  let { k, expanded, busy, ontoggle, onedit, onregenerate, ontoggledisabled, onrevoke }: Props = $props();

  const used = $derived((k.usage_this_month?.input_tokens ?? 0) + (k.usage_this_month?.output_tokens ?? 0));
  const restrictionCount = $derived(k.allowed_models.length + k.allowed_ips.length + (k.no_log ? 1 : 0));
</script>

<li class="key-card" class:revoked={k.status === 'revoked'}>
  <div class="key-top">
    <div class="key-name-cell">
      <span class="key-name">{k.name}</span>
      <code class="key-prefix">{k.prefix}…</code>
    </div>
    <Badge status={k.status === 'active' ? 'healthy' : k.status === 'disabled' ? 'cancelled' : 'failed'} label={keyStatusLabels[k.status]()} />
  </div>

  <div class="key-usage">
    <span class="sr-only">{m.key_table_usage()}</span>
    <div class:unlimited={k.monthly_token_limit == null} class="key-meter">
      <Meter
        value={used}
        limit={k.monthly_token_limit}
        valueText={k.monthly_token_limit == null
          ? formatNumber(used)
          : m.key_usage_summary({ used: formatNumber(used), limit: formatNumber(k.monthly_token_limit) })}
        unlimitedText={m.key_no_limit()}
      />
    </div>
  </div>

  <div class="key-meta">
    <div class="scope-list" aria-label={m.key_scopes()}>
      {#each k.scopes as scope}
        <span class="chip">{scope === 'data_image' ? m.key_scope_image() : m.key_scope_inference()}</span>
      {/each}
    </div>
    {#if restrictionCount === 0}
      <span class="no-restrictions">{m.key_no_restrictions()}</span>
    {:else}
      <span class="mono restriction-count">{restrictionCount}</span>
    {/if}
  </div>

  {#if expanded}
    <div class="key-detail">
      <dl class="key-dates">
        <div>
          <dt>{m.key_expires()}</dt>
          <dd title={k.expires_at ? formatDateTime(k.expires_at) : undefined}>{k.expires_at ? humanizeDate(k.expires_at) : m.key_never()}</dd>
        </div>
        <div>
          <dt>{m.key_created()}</dt>
          <dd>{formatDateTime(k.created_at)}</dd>
        </div>
        <div>
          <dt>{m.key_last_used()}</dt>
          <dd>{k.last_used_at ? formatDateTime(k.last_used_at) : m.key_never()}</dd>
        </div>
        <div>
          <dt>{m.key_usage_requests({ n: k.usage_this_month?.requests ?? 0 })}</dt>
          <dd>{#if k.requests_per_minute != null}{m.key_rpm_summary({ rpm: formatNumber(k.requests_per_minute) })}{:else}—{/if}</dd>
        </div>
      </dl>
      <div class="restriction-list" aria-label={m.key_restrictions()}>
        {#each k.allowed_models as model}
          <span class="chip mono">{m.key_models_count({ list: model })}</span>
        {/each}
        {#each k.allowed_ips as ip}
          <span class="chip mono">{m.key_ips_count({ list: ip })}</span>
        {/each}
        {#if k.no_log}<span class="chip">{m.key_no_log_summary()}</span>{/if}
        {#if restrictionCount === 0}
          <span class="no-restrictions">{m.key_no_restrictions()}</span>
        {/if}
      </div>
    </div>
  {/if}

  <div class="key-actions">
    <button type="button" class="detail-toggle" onclick={ontoggle} aria-expanded={expanded}>
      {expanded ? m.key_hide_details() : m.key_show_details()}
    </button>
    <span class="spacer"></span>
    {#if k.status !== 'revoked'}
      <Button variant="outline" size="sm" onclick={onedit} ariaLabel={`${m.key_edit()} ${k.name}`}>{m.key_edit()}</Button>
      <Button variant="outline" size="sm" disabled={busy} onclick={onregenerate} ariaLabel={`${m.key_regenerate()} ${k.name}`}>{m.key_regenerate()}</Button>
      {#if k.status === 'disabled'}
        <Button variant="outline" size="sm" disabled={busy} onclick={ontoggledisabled} ariaLabel={`${m.key_enable()} ${k.name}`}>{m.key_enable()}</Button>
      {:else}
        <Button variant="ghost" size="sm" disabled={busy} onclick={ontoggledisabled} ariaLabel={`${m.key_disable()} ${k.name}`}>{m.key_disable()}</Button>
      {/if}
      <Button variant="danger" size="sm" onclick={onrevoke} ariaLabel={`${m.key_revoke()} ${k.name}`}>{m.key_revoke()}</Button>
    {/if}
  </div>
</li>

<style>
  .key-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-5);
    background: var(--bg-surface);
    border: var(--border-w) solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-1);
    min-width: 0;
  }

  .key-card.revoked { background: var(--bg-inset); box-shadow: none; }
  .key-card.revoked .key-name { color: var(--text-2); }

  .key-top {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .key-name-cell {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .key-name {
    color: var(--text-1);
    font-size: var(--text-md);
    font-weight: var(--weight-semibold);
    overflow-wrap: anywhere;
  }

  .key-prefix {
    align-self: flex-start;
    color: var(--text-3);
    font-size: var(--text-xs);
  }

  .key-usage {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .key-meter { width: 100%; }
  .key-meter.unlimited :global(.meter-track) { display: none; }
  .key-meter :global(.meter-label) { display: none; }

  .key-meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .scope-list,
  .restriction-list {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .restriction-count {
    color: var(--text-2);
    font-size: var(--text-xs);
  }

  .no-restrictions {
    color: var(--text-3);
    font-size: var(--text-xs);
  }

  .key-actions {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-2);
    padding-top: var(--space-4);
    border-top: var(--border-w) solid var(--border);
  }

  .key-actions .spacer { flex: 1 1 auto; }

  .detail-toggle {
    background: transparent;
    border: var(--border-w) solid transparent;
    border-radius: var(--radius-sm);
    color: var(--text-2);
    font-size: var(--text-sm);
    min-height: var(--control-h-sm);
    padding: 0 var(--space-3);
    cursor: pointer;
  }

  .detail-toggle:hover {
    color: var(--text-1);
    background: var(--bg-hover);
  }

  .key-detail {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-4);
    background: var(--bg-inset);
    border-radius: var(--radius);
  }

  .key-dates {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, var(--col-sm)), 1fr));
    gap: var(--space-4);
    margin: 0;
  }

  .key-dates div { min-width: 0; }

  .key-dates dt {
    color: var(--text-3);
    font-size: var(--text-xs);
  }

  .key-dates dd {
    margin: var(--space-1) 0 0;
    color: var(--text-1);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    overflow-wrap: anywhere;
  }
</style>
