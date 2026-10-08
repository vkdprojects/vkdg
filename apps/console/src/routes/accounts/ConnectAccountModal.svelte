<script lang="ts">
  import { Dialog } from 'bits-ui';
  import { X as XIcon, ExternalLink, Check, AlertTriangle } from 'lucide-svelte';
  import { api } from '$lib/api.js';
  import type { Account, ConnectionOutcome, LoginMethod, LoginStart } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { Button, CopyButton, Select, Spinner } from '$lib/components/index.js';

  interface Props {
    open: boolean;
    /** Preselected provider (reauth). */
    provider?: string;
    /** Reconnect this account in place, keeping its id. */
    accountId?: string;
    onconnected: (account: Account, outcome: ConnectionOutcome) => void;
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
  let pkceAutoCapture = $state(false);
  /** No loopback callback available: the code is pasted, not captured. */
  let pkceManual = $state(false);
  // Plain variable, not `$state`: `stop()` reads it, and the open/reset `$effect`
  // below calls `stop()`. As state it became an effect dependency, so opening the
  // popup re-ran the effect, which reset the modal and cancelled the whole login.
  let popup: Window | null = null;

  const method = $derived(methods.find((x) => x.id === methodId));

  // Every async continuation checks its generation; bumping it cancels the flow.
  let gen = 0;
  let pollTimer: ReturnType<typeof setTimeout> | undefined;
  let tickTimer: ReturnType<typeof setInterval> | undefined;
  let oauthChannel: BroadcastChannel | null = null;

  function stop() {
    gen++;
    clearTimeout(pollTimer);
    clearInterval(tickTimer);
    try { oauthChannel?.close(); } catch { /* ignore */ }
    oauthChannel = null;
    try { popup?.close(); } catch { /* ignore */ }
    popup = null;
    pkceAutoCapture = false;
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

  function finish(res: { account: Account } & ConnectionOutcome) {
    stop();
    open = false;
    onconnected(res.account, res);
  }

  async function start(e: Event) {
    e.preventDefault();
    if (!method) return;
    const g = ++gen;
    busy = true;
    try {
      if (method.flow === 'import_token') {
        const res = await api.importToken(selected, method.id, filledParams(), accountId);
        if (g === gen) finish(res);
        return;
      }
      const startParams = filledParams();
      if (method.flow === 'authorization_code_pkce') {
        const redirect = loopbackRedirect();
        pkceManual = redirect === null;
        if (redirect) startParams.redirect_uri = redirect;
      }
      const res = await api.startLogin(selected, method.id, startParams, accountId);
      if (g !== gen) return;
      flow = res;
      if (res.flow === 'device_code') {
        step = 'device';
        secondsLeft = res.expires_in_secs;
        tickTimer = setInterval(() => {
          secondsLeft = Math.max(0, secondsLeft - 1);
          if (secondsLeft === 0) fail(m.acct_expired());
        }, 1000);
      } else {
        step = 'pkce';
        openPkcePopup(res.authorize_url, res.login_id, g);
      }
    } catch (err) {
      if (g === gen) fail((err as Error).message);
    } finally {
      if (g === gen) busy = false;
    }
  }

  /**
   * claude.ai only redirects to its own code page or to a loopback `/callback`.
   * On a loopback console the callback page captures the code automatically;
   * anywhere else the user copies the code from claude.ai's page and pastes it.
   */
  function loopbackRedirect(): string | null {
    const { protocol, hostname, origin } = window.location;
    if (protocol !== 'http:' || (hostname !== 'localhost' && hostname !== '127.0.0.1')) return null;
    return `${origin}/callback`;
  }

  function openPkcePopup(authorizeUrl: string, loginId: string, g: number) {
    const w = 520, h = 700;
    const left = Math.round(window.screenX + (window.outerWidth - w) / 2);
    const top  = Math.round(window.screenY + (window.outerHeight - h) / 2);
    const opened = window.open(
      authorizeUrl,
      'vkdg_oauth',
      `width=${w},height=${h},left=${left},top=${top},toolbar=0,menubar=0,location=1`,
    );
    popup = opened;

    // Blocked popup, or a non-loopback console with no callback to capture: the user pastes the code.
    if (!opened || pkceManual) {
      pkceAutoCapture = false;
      return;
    }
    // The /callback page posts the code on this channel. It is the only capture
    // path: claude.ai serves a Cross-Origin-Opener-Policy that severs `window.opener`
    // and makes `popup.closed` read true while the popup is still open, so neither
    // postMessage nor popup polling can be relied on.
    try { oauthChannel?.close(); } catch { /* ignore */ }
    try {
      oauthChannel = new BroadcastChannel('vkdg_oauth_callback');
      oauthChannel.onmessage = (ev) => {
        if (g !== gen) return;
        handleOAuthMessage(ev.data, loginId, g);
      };
    } catch {
      pkceAutoCapture = false;
      return;
    }

    pkceAutoCapture = true;
  }

  function handleOAuthMessage(data: Record<string, string>, loginId: string, g: number) {
    if (data?.source !== 'vkdg_oauth_callback') return;
    if (data.type === 'oauth_error') { fail(data.error_description || data.error || 'OAuth error'); return; }
    if (data.type !== 'oauth_code' || !data.code) return;
    pkceCode = data.code;
    pkceAutoCapture = false;
    exchangePkceCode(loginId, data.code, g);
  }

  async function exchangePkceCode(loginId: string, code: string, g: number) {
    if (g !== gen) return;
    busy = true;
    try {
      const res = await api.pollLogin(selected, loginId, code.trim());
      if (g !== gen) return;
      if (res.status === 'done') finish(res);
      else if (res.status === 'failed') fail(res.message);
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
        if (res.status === 'done') finish(res);
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
      if (res.status === 'done') finish(res);
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

      <div class="dialog-body body" aria-live="polite">
        {#if step === 'pick'}
          <form onsubmit={start} class="fields">
            <Select label={m.acct_provider()} options={providers} bind:value={selected} disabled={busy} />

            {#if loadingMethods}
              <div class="muted status-row"><Spinner size="sm" /> {m.common_loading()}</div>
            {:else if methods.length === 0}
              <p class="muted status-row">{m.acct_no_methods()}</p>
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
            <div class="dialog-footer">
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
          <div class="muted status-row" role="status">
            <Spinner size="sm" /> {m.acct_waiting()}
          </div>
          <!-- Countdown updates every second; keep it out of the live region to avoid chatter. -->
          <p class="muted mono status-row" aria-live="off">{m.acct_expires_in({ time: fmtTime(secondsLeft) })}</p>
          <div class="dialog-footer">
            <Button variant="outline" onclick={() => (open = false)}>{m.common_cancel()}</Button>
          </div>
        {:else if step === 'pkce' && flow?.flow === 'authorization_code_pkce'}
          {#if pkceAutoCapture}
            <!-- Popup is open — waiting for automatic code capture. -->
            <div class="pkce-auto">
              <div class="pkce-icon spin">⟳</div>
              <p class="step-intro">Authorize in the popup window…</p>
              <p class="muted status-row">The window opened at claude.ai. After you sign in, this dialog completes automatically.</p>
            </div>
            <div class="dialog-footer">
              <Button variant="outline" onclick={() => (pkceAutoCapture = false)}>{m.acct_enter_code_manually()}</Button>
              <Button variant="outline" onclick={() => (open = false)}>{m.common_cancel()}</Button>
            </div>
          {:else if busy}
            <!-- Code received, exchange in progress. -->
            <div class="pkce-auto">
              <Spinner size="lg" />
              <p class="step-intro">Completing sign-in…</p>
            </div>
          {:else}
            <!-- Manual paste: no loopback callback, blocked popup, or the user asked for it. -->
            {@const authorizeUrl = (flow as { authorize_url: string }).authorize_url}
            <div class="pkce-section">
              <p class="step-intro">{m.acct_pkce_step()}</p>
              <div class="pkce-url-row">
                <code class="pkce-url" title={authorizeUrl}>{authorizeUrl.slice(0, 60)}…</code>
                <CopyButton text={authorizeUrl} />
              </div>
              <Button
                variant="outline"
                onclick={() => openPkcePopup(authorizeUrl, flow!.login_id, gen)}
              >
                <ExternalLink size={14} aria-hidden="true" />
                Reopen popup
              </Button>
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
              <div class="dialog-footer">
                <Button variant="outline" onclick={() => (open = false)}>{m.common_cancel()}</Button>
                <Button type="submit" disabled={busy || !pkceCode.trim()}>
                  {#if busy}<Spinner size="sm" />{/if}
                  {m.acct_submit_code()}
                </Button>
              </div>
            </form>
          {/if}
        {:else if step === 'error'}
          <div class="error" role="alert">
            <AlertTriangle size={16} aria-hidden="true" />
            <p>{errorMsg}</p>
          </div>
          <div class="dialog-footer">
            <Button variant="outline" onclick={() => (open = false)}>{m.acct_close()}</Button>
            <Button onclick={() => { reset(); loadMethods(selected); }}>{m.acct_retry()}</Button>
          </div>
        {/if}
      </div>
    </Dialog.Content>
  </Dialog.Portal>
</Dialog.Root>

<style>
  .dialog-heading {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .step-tag {
    color: var(--text-2);
    font-size: var(--text-xs);
    padding: var(--space-0) var(--space-2);
    border: var(--border-w) solid var(--border);
    border-radius: var(--radius-full);
    background: var(--bg-elevated);
    text-transform: lowercase;
  }

  .body {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    flex: 1 1 auto;
    min-height: 0;
  }

  /* Footers sit inside the padded body: plain button row, no extra chrome. */
  .body :global(.dialog-footer) {
    padding: 0;
    border-top: 0;
  }

  .status-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    margin: 0;
  }

  .step-intro {
    color: var(--text-2);
    font-size: var(--text-base);
    line-height: var(--leading);
    margin: 0;
  }

  .code-box {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    background: var(--bg-inset);
    border: var(--border-w) solid var(--border-strong);
    border-radius: var(--radius);
    padding: var(--space-4);
  }

  .code-eyebrow {
    color: var(--text-3);
    font-size: var(--text-xs);
  }

  .code-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    flex-wrap: wrap;
  }

  .user-code {
    font-size: var(--text-2xl);
    font-weight: var(--weight-bold);
    letter-spacing: var(--tracking-dot);
    color: var(--text-1);
    overflow-wrap: anywhere;
  }

  .verify-link {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    min-height: var(--control-h);
    color: var(--accent);
    font-size: var(--text-base);
    font-weight: var(--weight-medium);
  }

  .error {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    color: var(--danger);
    background: var(--danger-subtle);
    border: var(--border-w) solid color-mix(in oklch, var(--danger) 34%, transparent);
    border-radius: var(--radius);
    padding: var(--space-3) var(--space-4);
  }

  .error :global(svg) {
    flex-shrink: 0;
    margin-top: var(--space-0);
  }

  .error p {
    font-size: var(--text-base);
    margin: 0;
    color: var(--text-1);
    overflow-wrap: anywhere;
  }

  .method-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, var(--col-sm)), 1fr));
    gap: var(--space-2);
  }

  .method-card {
    position: relative;
    align-items: flex-start;
    background: var(--bg-elevated);
    border: var(--border-w) solid var(--border);
    border-radius: var(--radius);
    color: inherit;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-height: var(--control-h-lg);
    padding: var(--space-3) var(--space-4);
    text-align: left;
    width: 100%;
  }

  .method-card:hover {
    border-color: var(--accent);
  }

  .method-card.selected {
    border-color: var(--accent);
    background: var(--accent-subtle);
    box-shadow: 0 0 0 var(--border-w) var(--accent);
  }

  :global(.method-check) {
    position: absolute;
    top: var(--space-2);
    right: var(--space-2);
    color: var(--accent);
  }

  .method-icon {
    font-size: var(--text-lg);
    line-height: 1;
  }

  .method-label {
    color: var(--text-1);
    font-size: var(--text-base);
    font-weight: var(--weight-semibold);
    padding-right: var(--space-4);
  }

  .method-hint {
    color: var(--text-3);
    font-size: var(--text-xs);
    line-height: var(--leading);
  }

  .social-choice {
    border: none;
    margin: 0;
    padding: 0;
    min-width: 0;
  }

  .social-choice .field-label {
    margin-bottom: var(--space-2);
    display: block;
    padding: 0;
  }

  .social-options {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .social-opt {
    position: relative;
    flex-direction: row;
    align-items: center;
    background: var(--bg-elevated);
    border: var(--border-w) solid var(--border);
    border-radius: var(--radius);
    cursor: pointer;
    display: flex;
    gap: var(--space-2);
    min-height: var(--control-h);
    padding: 0 var(--space-4);
    font-size: var(--text-base);
    font-weight: var(--weight-medium);
    color: var(--text-2);
  }

  .social-opt input[type='radio'] {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }

  .social-opt:hover { border-color: var(--accent); }

  .social-opt:has(input:focus-visible) {
    outline: var(--focus-w) solid var(--accent);
    outline-offset: var(--focus-w);
  }

  .social-opt.checked {
    border-color: var(--accent);
    background: var(--accent-subtle);
    color: var(--text-1);
  }

  :global(.social-check) {
    color: var(--accent);
  }

  .pkce-section {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-3);
  }

  .pkce-url-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
    max-width: 100%;
    min-width: 0;
  }

  .pkce-url {
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .pkce-auto {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3) 0;
    text-align: center;
  }

  .pkce-icon {
    font-size: var(--text-2xl);
    line-height: 1;
  }

  .pkce-icon.spin {
    animation: spin calc(var(--dur-3) * 3) linear infinite;
    color: var(--accent);
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  @media (prefers-reduced-motion: reduce) {
    .pkce-icon.spin { animation: none; }
  }
</style>
