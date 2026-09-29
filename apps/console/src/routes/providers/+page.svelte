<script lang="ts">
  import { onMount } from 'svelte';
  import { ExternalLink, ArrowRight } from 'lucide-svelte';
  import { api } from '$lib/api.js';
  import type { Account, ConnectionSummary, OAuthProvider } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { Badge, EmptyState, Spinner, StatusDot } from '$lib/components/index.js';
  type Filter = 'all' | 'oauth_ide' | 'llm_api' | 'compatible';
  let providers=$state<OAuthProvider[]>([]), accounts=$state<Account[]>([]), connections=$state<ConnectionSummary[]>([]), loading=$state(true), filter=$state<Filter>('all');
  const filtered=$derived(filter==='all'?providers:providers.filter(p=>p.category===filter));
  const statusLabels:Record<string,()=>string>={healthy:m.connection_status_healthy,degraded:m.connection_status_degraded,circuit_open:m.connection_status_circuit_open,cooldown:m.connection_status_cooldown,unknown:m.connection_status_unknown};
  const categoryLabels:Record<string,()=>string>={oauth_ide:m.providers_filter_oauth_ide,llm_api:m.providers_filter_llm_api,compatible:m.providers_filter_compatible};
  const filters:{value:Filter;label:()=>string}[]=[{value:'all',label:m.providers_filter_all},{value:'oauth_ide',label:m.providers_filter_oauth_ide},{value:'llm_api',label:m.providers_filter_llm_api},{value:'compatible',label:m.providers_filter_compatible}];
  function accountsFor(id:string){return accounts.filter(a=>a.provider===id)}
  function connectionsFor(id:string){return connections.filter(c=>c.provider===id)}
  function dominantStatus(id:string){const cs=connectionsFor(id);return cs.find(c=>c.status==='circuit_open')?.status??cs.find(c=>c.status==='degraded')?.status??cs.find(c=>c.status==='cooldown')?.status??cs[0]?.status??'unknown'}
  onMount(async()=>{try{const [p,a,c]=await Promise.all([api.oauthProviders(),api.listAccounts(),api.listConnections()]);providers=p.items;accounts=a.items;connections=c.items}finally{loading=false}});
</script>
<div class="page inventory">
  <div class="page-header"><div><h1>{m.providers_title()}</h1><p>{m.providers_inventory_subtitle()}</p></div></div>
  <div class="filters" role="tablist" aria-label={m.providers_filter_label()}>{#each filters as f}<button role="tab" aria-selected={filter===f.value} class:active={filter===f.value} onclick={()=>filter=f.value}>{f.label()}</button>{/each}</div>
  {#if loading}<div class="loading"><Spinner size="sm"/>{m.common_loading()}</div>
  {:else if filtered.length===0}<EmptyState title={m.providers_empty()} description={m.providers_connect_first()}/>
  {:else}<div class="table-wrap"><table><thead><tr><th>{m.connection_provider()}</th><th>{m.providers_category()}</th><th>{m.provider_detail_accounts()}</th><th>{m.nav_connections()}</th><th>{m.connection_status()}</th><th>{m.gateway_concurrency()}</th><th><span class="sr-only">{m.common_actions()}</span></th></tr></thead><tbody>
    {#each filtered as p (p.id)}
      {@const pa=accountsFor(p.id)}{@const pc=connectionsFor(p.id)}{@const status=dominantStatus(p.id)}
      <tr><td><div class="provider-cell"><span class="icon" style:background={p.icon_color}>{p.icon_char}</span><div><a class="provider-link" href="/providers/{encodeURIComponent(p.id)}">{p.display_name}</a>{#if p.site_url}<a class="site" href={p.site_url} target="_blank" rel="noopener noreferrer" aria-label={m.providers_open_site({name:p.display_name})}><ExternalLink size={12}/></a>{/if}<small>{p.description??p.id}</small></div></div></td>
      <td>{(categoryLabels[p.category]??(()=>p.category))()}</td>
      <td><strong class="mono">{pa.filter(a=>a.status==='active').length}</strong> {m.providers_active_short()}{#if pa.some(a=>a.status==='needs_login')}<span class="auth-alert">{pa.filter(a=>a.status==='needs_login').length} {m.acct_status_needs_login()}</span>{/if}</td>
      <td class="mono">{pc.length}</td><td><div class="status"><StatusDot {status}/><Badge {status} label={(statusLabels[status]??m.connection_status_unknown)()}/></div></td>
      <td class="mono">{pc.reduce((n,c)=>n+c.active_requests,0)} / {pc.reduce((n,c)=>n+c.max_concurrent,0)||'—'}</td><td><a class="open" href="/providers/{encodeURIComponent(p.id)}" aria-label={m.providers_open_detail({name:p.display_name})}><ArrowRight size={15}/></a></td></tr>
    {/each}
  </tbody></table></div>{/if}
</div>
<style>
  .page-header h1{margin:0 0 3px}.page-header p{margin:0;font-size:var(--text-sm)}.filters{display:flex;gap:4px;margin:18px 0 12px}.filters button{background:var(--bg-surface);border:1px solid var(--border);color:var(--text-2);padding:5px 11px;cursor:pointer;font-size:var(--text-xs)}.filters button.active{color:var(--text-1);border-color:var(--accent);box-shadow:inset 0 -2px var(--accent)}.loading{display:flex;gap:8px;align-items:center;color:var(--text-3)}.table-wrap{overflow-x:auto;border:1px solid var(--border)}table{min-width:850px}.provider-cell{display:flex;align-items:center;gap:10px;min-width:235px}.icon{width:30px;height:30px;border-radius:4px;display:grid;place-items:center;color:#fff;font-weight:700}.provider-link{color:var(--accent);text-decoration:none;font-weight:550}
</style>
