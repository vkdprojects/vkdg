<script lang="ts">
  import { Dialog } from 'bits-ui';
  import { X as XIcon, ExternalLink, Check, AlertTriangle } from 'lucide-svelte';
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
        <div class="dialog-heading">
          <Dialog.Title class="dialog-title">{m.acct_connect()}</Dialog.Title>
          {#if step !== 'pick'}
            <span class="step-tag mono">{selected}</span>
          {/if}
        </div>
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
                <!-- Cards per método: mais claro que um dropdown quando >1 opção -->
                <div class="method-grid">
                  {#each methods as mx (mx.id)}
                    <button
                      type="button"
                      class="method-card"
                      class:selected={methodId === mx.id}
                      onclick={() => pickMethod(mx.id)}
                      aria-pressed={methodId === mx.id}
                    >
                      {#if methodId === mx.id}
                        <Check class="method-check" size={14} aria-hidden="true" />
                      {/if}
                      {#if mx.icon_char}
                        <span class="method-icon">{mx.icon_char}</span>
                      {/if}
                      <span class="method-label">{mx.label}</span>
                      {#if mx.hint}
                        <span class="method-hint">{mx.hint}</span>
                      {/if}
                    </button>
                  {/each}
                </div>
              {/if}
              {#each method?.fields ?? [] as f (f.id)}
                {#if f.id === 'provider' && method?.id === 'social'}
                  <!-- Kiro social: Google vs GitHub como radio, não campo livre -->
                  <fieldset class="social-choice">
                    <legend class="field-label">{f.label}</legend>
                    <div class="social-options">
                      {#each ['Google', 'Github'] as opt (opt)}
                        <label class="social-opt" class:checked={params[f.id] === opt}>
                          <input
                            type="radio"
                            name="social-provider"
                            value={opt}
                            bind:group={params[f.id]}
                          />
                          {#if params[f.id] === opt}
                            <Check class="social-check" size={12} aria-hidden="true" />
                          {/if}
                          {opt}
                        </label>
                      {/each}
                    </div>
                  </fieldset>
                {:else}
                  <div class="field">
                    <label for="login-{f.id}">{f.label}{#if f.required}<span aria-hidden="true"> *</span>{/if}</label>
                    <input
                      id="login-{f.id}"
                      type={f.secret ? 'password' : 'text'}
                      autocomplete="off"
                      required={f.required}
                      placeholder={f.placeholder ?? ''}
                      bind:value={params[f.id]}
                    />
                  </div>
                {/if}
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
          <p class="step-intro">{m.acct_device_step()}</p>
          <div class="code-box">
            <span class="code-eyebrow mono">{m.acct_copy_code()}</span>
            <div class="code-row">
              <span class="user-code mono">{flow.user_code}</span>
              <CopyButton text={flow.user_code} label={m.acct_copy_code()} />
            </div>
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
          <p class="muted mono" aria-live="off">{m.acct_expires_in({ time: fmtTime(secondsLeft) })}</p>
          <div class="footer">
            <Button variant="outline" onclick={() => (open = false)}>{m.common_cancel()}</Button>
          </div>
        {:else if step === 'pkce' && flow?.flow === 'authorization_code_pkce'}
          {@const authorizeUrl = (flow as { authorize_url: string }).authorize_url}
          <div class="pkce-section">
            <p class="step-intro">{m.acct_pkce_step()}</p>
            <Button
              onclick={() => window.open(authorizeUrl, '_blank', 'noopener,noreferrer')}
            >
              <ExternalLink size={16} aria-hidden="true" />
              {m.acct_open_authorize()}
            </Button>
            <div class="pkce-url-row">
              <code class="pkce-url" title={authorizeUrl}>{authorizeUrl.slice(0, 60)}…</code>
              <CopyButton text={authorizeUrl} />
            </div>
            <p class="pkce-waiting-hint">
              <Spinner size="sm" />
              {m.acct_waiting()}
            </p>
          </div>
          <form onsubmit={submitCode} class="fields">
            <div class="field">
              <label for="pkce-code">{m.acct_code_label()}</label>
              <input
                id="pkce-code"
                type="text"
                autocomplete="off"
                required
                placeholder="Paste the code or full callback URL"
                bind:value={pkceCode}
              />
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
          <div class="error" role="alert">
            <AlertTriangle size={16} aria-hidden="true" />
            <p>{errorMsg}</p>
          </div>
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

  .dialog-heading {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }

  .step-tag {
    color: var(--text-3);
    font-size: var(--text-xs);
    padding: 0.0625rem 0.375rem;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    text-transform: lowercase;
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

  .step-intro {
    color: var(--text-2);
    font-size: 0.875rem;
    margin: 0;
  }

  .code-box {
    display: flex;
    flex-direction: column;
    gap: 8px;
    background: var(--bg-base);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 14px 16px;
  }

  .code-eyebrow {
    color: var(--text-3);
    font-size: var(--text-2xs);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .code-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .user-code {
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
    display: flex;
    align-items: flex-start;
    gap: 8px;
    color: var(--danger);
    background: var(--danger-subtle);
    border: 1px solid color-mix(in oklch, var(--danger) 34%, transparent);
    border-radius: var(--radius-sm);
    padding: 10px 12px;
  }

  .error :global(svg) {
    flex-shrink: 0;
    margin-top: 0.125rem;
  }

  .error p {
    font-size: 0.875rem;
    margin: 0;
    color: var(--text-1);
  }


  .method-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
    gap: 8px;
  }

  .method-card {
    position: relative;
    align-items: flex-start;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 12px;
    text-align: left;
    transition: border-color 0.12s;
    width: 100%;
  }

  .method-card:hover {
    border-color: var(--accent);
  }


  .method-card.selected {
    border-color: var(--accent);
    background: var(--accent-subtle);
  }

  :global(.method-check) {
    position: absolute;
    top: 10px;
    right: 10px;
    color: var(--accent);
  }


  .method-icon {
    font-size: 1.25rem;
    line-height: 1;
  }

  .method-label {
    color: var(--text-1);
    font-size: 0.875rem;
    font-weight: 600;
  }

  .method-hint {
    color: var(--text-3);
    font-size: 0.75rem;
    line-height: 1.3;
  }

  .social-choice {
    border: none;
    margin: 0;
    padding: 0;
  }

  .field-label {
    color: var(--text-2);
    font-size: 0.8125rem;
    font-weight: 500;
    margin-bottom: 0.5rem;
    display: block;
  }

  .social-options {
    display: flex;
    gap: 8px;
  }

  .social-opt {
    align-items: center;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    cursor: pointer;
    display: flex;
    gap: 6px;
    padding: 8px 14px;
    font-size: 0.875rem;
    font-weight: 500;
    color: var(--text-2);
    transition: border-color 0.12s;
  }

  .social-opt input[type='radio'] {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }

  .social-opt:hover { border-color: var(--accent); }

  .social-opt.checked {
    border-color: var(--accent);
    background: var(--accent-subtle);
    color: var(--text-1);
  }

  :global(.social-check) {
    color: var(--accent);
  }
</style>
