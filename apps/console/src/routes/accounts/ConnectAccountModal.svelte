<script lang="ts">
  import { Dialog } from 'bits-ui';
  import { X as XIcon, ExternalLink } from 'lucide-svelte';
  import { api } from '$lib/api.js';
  import type { Account, LoginMethod, LoginStart } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { Button, CopyButton, Select, Spinner } from '$lib/components/index.js';

  interface Props {
    open: boolean;
    /** Preselected provider (reauth). */
    provider?: string;
    /** Reconnect this account in place, keeping its id. */
    accountId?: string;
    onconnected: (account: Account) => void;
  }
  let { open = $bindable(), provider = 'kiro', accountId, onconnected }: Props = $props();

  // Loaded from the gateway: what is registered here, OAuth plugins included.
  let providers = $state<{ value: string; label: string }[]>([]);

  type Step = 'pick' | 'device' | 'pkce' | 'error';

  let selected = $state('');
  let methods = $state<LoginMethod[]>([]);
  let methodId = $state('');
  let params = $state<Record<string, string>>({});
  let loadingMethods = $state(false);
  let busy = $state(false);
  let step = $state<Step>('pick');
  let errorMsg = $state('');
  let flow = $state<LoginStart | null>(null);
  let secondsLeft = $state(0);
  let pkceCode = $state('');

  const method = $derived(methods.find((x) => x.id === methodId));

  // Every async continuation checks its generation; bumping it cancels the flow.
  let gen = 0;
  let pollTimer: ReturnType<typeof setTimeout> | undefined;
  let tickTimer: ReturnType<typeof setInterval> | undefined;

  function stop() {
    gen++;
    clearTimeout(pollTimer);
    clearInterval(tickTimer);
  }

  function fail(msg: string) {
    stop();
    errorMsg = msg;
    step = 'error';
  }

  function reset() {
    stop();
    step = 'pick';
    errorMsg = '';
    flow = null;
    pkceCode = '';
    busy = false;
  }

  // Opening resets state and preselects the provider; closing stops polling.
  $effect(() => {
    if (open) {
      reset();
      selected = provider;
    } else {
      stop();
    }
    return stop;
  });

  $effect(() => {
    if (!open || providers.length) return;
    api
      .oauthProviders()
      .then((res) => {
        providers = res.items.map((p) => ({ value: p.id, label: p.display_name }));
        if (!providers.some((p) => p.value === selected)) selected = providers[0]?.value ?? '';
      })
      .catch((e) => fail((e as Error).message));
  });

  $effect(() => {
    if (open && selected) loadMethods(selected);
  });

  async function loadMethods(p: string) {
    const g = ++gen;
    loadingMethods = true;
    methods = [];
    methodId = '';
    try {
      const res = await api.loginMethods(p);
      if (g !== gen) return;
      methods = res.items;
      if (methods[0]) pickMethod(methods[0].id);
    } catch (e) {
      if (g === gen) fail((e as Error).message);
    } finally {
      if (g === gen) loadingMethods = false;
    }
  }

  function pickMethod(id: string) {
    methodId = id;
    const found = methods.find((x) => x.id === id);
    params = Object.fromEntries((found?.fields ?? []).map((f) => [f.id, f.default ?? '']));
  }

  function filledParams(): Record<string, string> {
    return Object.fromEntries(Object.entries(params).filter(([, v]) => v.trim() !== ''));
  }

  function finish(account: Account) {
    stop();
    open = false;
    onconnected(account);
  }

  async function start(e: Event) {
    e.preventDefault();
    if (!method) return;
    const g = ++gen;
    busy = true;
    try {
      if (method.flow === 'import_token') {
        const res = await api.importToken(selected, method.id, filledParams(), accountId);
        if (g === gen) finish(res.account);
        return;
      }
      const res = await api.startLogin(selected, method.id, filledParams(), accountId);
      if (g !== gen) return;
      flow = res;
      if (res.flow === 'device_code') {
        step = 'device';
        secondsLeft = res.expires_in_secs;
        tickTimer = setInterval(() => {
          secondsLeft = Math.max(0, secondsLeft - 1);
          if (secondsLeft === 0) fail(m.acct_expired());
        }, 1000);
        schedulePoll(g, res.login_id, res.interval_secs);
      } else {
        step = 'pkce';
        window.open(res.authorize_url, '_blank', 'noopener');
      }
    } catch (err) {
      if (g === gen) fail((err as Error).message);
    } finally {
      if (g === gen) busy = false;
    }
  }

  function schedulePoll(g: number, loginId: string, intervalSecs: number) {
    pollTimer = setTimeout(async () => {
      if (g !== gen) return;
      try {
        const res = await api.pollLogin(selected, loginId);
        if (g !== gen) return;
        if (res.status === 'done') finish(res.account);
        else if (res.status === 'failed') fail(res.message);
        // RFC 8628: slow_down adds 5 seconds to the interval.
        else schedulePoll(g, loginId, res.status === 'slow_down' ? intervalSecs + 5 : intervalSecs);
      } catch (err) {
        if (g === gen) fail((err as Error).message);
      }
    }, Math.max(1, intervalSecs) * 1000);
  }

  async function submitCode(e: Event) {
    e.preventDefault();
    if (!flow || !pkceCode.trim()) return;
    const g = gen;
    busy = true;
    try {
      const res = await api.pollLogin(selected, flow.login_id, pkceCode.trim());
      if (g !== gen) return;
      if (res.status === 'done') finish(res.account);
      else if (res.status === 'failed') fail(res.message);
    } catch (err) {
      if (g === gen) fail((err as Error).message);
    } finally {
      if (g === gen) busy = false;
    }
  }

  function fmtTime(s: number): string {
    return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
  }
</script>

<Dialog.Root bind:open>
  <Dialog.Portal>
    <Dialog.Overlay class="dialog-overlay" />
    <Dialog.Content class="dialog-content" aria-describedby={undefined}>
      <div class="dialog-header">
        <Dialog.Title class="dialog-title">{m.acct_connect()}</Dialog.Title>
        <button class="dialog-close" aria-label={m.acct_close()} onclick={() => (open = false)}>
          <XIcon size={16} aria-hidden="true" />
        </button>
      </div>

      <div class="body" aria-live="polite">
        {#if step === 'pick'}
          <form onsubmit={start} class="fields">
            <Select label={m.acct_provider()} options={providers} bind:value={selected} disabled={busy} />

            {#if loadingMethods}
              <div class="muted"><Spinner size="sm" /> {m.common_loading()}</div>
            {:else if methods.length === 0}
              <p class="muted">{m.acct_no_methods()}</p>
            {:else}
              {#if methods.length > 1}
                <Select
                  label={m.acct_method()}
                  options={methods.map((x) => ({ value: x.id, label: x.label }))}
                  value={methodId}
                  onchange={pickMethod}
                  disabled={busy}
                />
              {/if}
              {#each method?.fields ?? [] as f (f.id)}
                <div class="field">
                  <label for="login-{f.id}">{f.label}{#if f.required}<span aria-hidden="true"> *</span>{/if}</label>
                  <input
                    id="login-{f.id}"
                    type={f.secret ? 'password' : 'text'}
                    autocomplete="off"
                    required={f.required}
                    bind:value={params[f.id]}
                  />
                </div>
              {/each}
            {/if}

            <div class="footer">
              <Button variant="outline" onclick={() => (open = false)}>{m.common_cancel()}</Button>
              <Button type="submit" disabled={busy || !method}>
                {#if busy}<Spinner size="sm" />{/if}
                {m.acct_start()}
              </Button>
            </div>
          </form>
        {:else if step === 'device' && flow?.flow === 'device_code'}
          <p>{m.acct_device_step()}</p>
          <div class="code-box">
            <span class="user-code">{flow.user_code}</span>
            <CopyButton text={flow.user_code} label={m.acct_copy_code()} />
          </div>
          <a
            class="verify-link"
            href={flow.verification_uri_complete ?? flow.verification_uri}
            target="_blank"
            rel="noopener noreferrer"
          >
            <ExternalLink size={14} aria-hidden="true" />
            {m.acct_open_verification()}
          </a>
          <div class="muted" role="status">
            <Spinner size="sm" /> {m.acct_waiting()}
          </div>
          <!-- Countdown updates every second; keep it out of the live region to avoid chatter. -->
          <p class="muted" aria-live="off">{m.acct_expires_in({ time: fmtTime(secondsLeft) })}</p>
          <div class="footer">
            <Button variant="outline" onclick={() => (open = false)}>{m.common_cancel()}</Button>
          </div>
        {:else if step === 'pkce' && flow?.flow === 'authorization_code_pkce'}
          <p>{m.acct_pkce_step()}</p>
          <a class="verify-link" href={flow.authorize_url} target="_blank" rel="noopener noreferrer">
            <ExternalLink size={14} aria-hidden="true" />
            {m.acct_open_authorize()}
          </a>
          <form onsubmit={submitCode} class="fields">
            <div class="field">
              <label for="pkce-code">{m.acct_code_label()}</label>
              <input id="pkce-code" type="text" autocomplete="off" required bind:value={pkceCode} />
            </div>
            <div class="footer">
              <Button variant="outline" onclick={() => (open = false)}>{m.common_cancel()}</Button>
              <Button type="submit" disabled={busy || !pkceCode.trim()}>
                {#if busy}<Spinner size="sm" />{/if}
                {m.acct_submit_code()}
              </Button>
            </div>
          </form>
        {:else if step === 'error'}
          <p class="error" role="alert">{errorMsg}</p>
          <div class="footer">
            <Button variant="outline" onclick={() => (open = false)}>{m.acct_close()}</Button>
            <Button onclick={() => { reset(); loadMethods(selected); }}>{m.acct_retry()}</Button>
          </div>
        {/if}
      </div>
    </Dialog.Content>
  </Dialog.Portal>
</Dialog.Root>

<style>
  :global(.dialog-overlay) {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.45);
    z-index: 50;
  }

  :global(.dialog-content) {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    z-index: 51;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    width: min(480px, calc(100vw - 32px));
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.25);
  }

  :global(.dialog-title) {
    font-size: 1rem;
    font-weight: 600;
    color: var(--text-1);
    margin: 0;
  }

  .dialog-close {
    display: flex;
    background: transparent;
    border: none;
    color: var(--text-3);
    cursor: pointer;
    padding: 4px;
    border-radius: var(--radius-sm);
  }

  .dialog-close:hover {
    color: var(--text-1);
    background: var(--bg-hover);
  }

  .dialog-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 20px 20px 0;
  }

  .body {
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .fields {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .field label {
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--text-2);
  }

  .field input {
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--text-1);
    font-size: 0.875rem;
    padding: 0.4375rem 0.625rem;
  }

  .field input:focus {
    border-color: var(--accent);
    outline: none;
  }

  .footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .code-box {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    background: var(--bg-base);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 14px 16px;
  }

  .user-code {
    font-family: ui-monospace, 'SF Mono', Menlo, monospace;
    font-size: 1.75rem;
    font-weight: 700;
    letter-spacing: 0.12em;
    color: var(--text-1);
  }

  .verify-link {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--accent);
    font-size: 0.875rem;
    font-weight: 500;
  }

  .muted {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-3);
    font-size: 0.8125rem;
    margin: 0;
  }

  .error {
    color: var(--danger);
    font-size: 0.875rem;
    margin: 0;
  }
</style>
