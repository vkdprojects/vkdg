<script lang="ts">
  import { Dialog } from 'bits-ui';
  import { X as XIcon } from 'lucide-svelte';
  import { api } from '$lib/api.js';
  import type { Account, ConnectionOutcome, LoginMethod, LoginStart } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import StepDeviceCode from './StepDeviceCode.svelte';
  import StepError from './StepError.svelte';
  import StepPickMethod from './StepPickMethod.svelte';
  import StepPkce from './StepPkce.svelte';
  import { createPkceCapture, loopbackRedirect } from './pkcePopup.js';

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
  // Plain object, not `$state`: `stop()` closes it, and the open/reset `$effect`
  // below calls `stop()`. As state it became an effect dependency, so opening the
  // popup re-ran the effect, which reset the modal and cancelled the whole login.
  const capture = createPkceCapture();

  const method = $derived(methods.find((x) => x.id === methodId));

  // Every async continuation checks its generation; bumping it cancels the flow.
  let gen = 0;
  let tickTimer = 0;

  function stop() {
    gen++;
    window.clearInterval(tickTimer);
    capture.close();
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
        tickTimer = window.setInterval(() => {
          secondsLeft = Math.max(0, secondsLeft - 1);
          if (secondsLeft === 0) fail(m.acct_expired());
        }, 1000);
      } else {
        step = 'pkce';
        openPkce(res.authorize_url, res.login_id, g);
      }
    } catch (err) {
      if (g === gen) fail((err as Error).message);
    } finally {
      if (g === gen) busy = false;
    }
  }

  function openPkce(authorizeUrl: string, loginId: string, g: number) {
    pkceAutoCapture = capture.open(authorizeUrl, pkceManual, (r) => {
      if (g !== gen) return;
      if ('error' in r) { fail(r.error); return; }
      pkceCode = r.code;
      pkceAutoCapture = false;
      exchangePkceCode(loginId, r.code, g);
    });
  }

  /** Exchange a PKCE code (captured or pasted) while showing the busy state. */
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

  function submitCode(e: Event) {
    e.preventDefault();
    if (!flow || !pkceCode.trim()) return;
    void exchangePkceCode(flow.login_id, pkceCode, gen);
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
          <StepPickMethod
            {providers}
            bind:selected
            {methods}
            {methodId}
            {method}
            bind:params
            {loadingMethods}
            {busy}
            onpick={pickMethod}
            onsubmit={start}
            oncancel={() => (open = false)}
          />
        {:else if step === 'device' && flow?.flow === 'device_code'}
          <StepDeviceCode
            userCode={flow.user_code}
            verificationUrl={flow.verification_uri_complete ?? flow.verification_uri}
            {secondsLeft}
            oncancel={() => (open = false)}
          />
        {:else if step === 'pkce' && flow?.flow === 'authorization_code_pkce'}
          {@const pkce = flow}
          <StepPkce
            authorizeUrl={pkce.authorize_url}
            autoCapture={pkceAutoCapture}
            {busy}
            bind:code={pkceCode}
            onreopen={() => openPkce(pkce.authorize_url, pkce.login_id, gen)}
            onmanual={() => (pkceAutoCapture = false)}
            onsubmit={submitCode}
            oncancel={() => (open = false)}
          />
        {:else if step === 'error'}
          <StepError
            message={errorMsg}
            onclose={() => (open = false)}
            onretry={() => { reset(); loadMethods(selected); }}
          />
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

  /* Shared by every step component rendered inside the body. */
  .body :global(.status-row) {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    margin: 0;
  }

  .body :global(.step-intro) {
    color: var(--text-2);
    font-size: var(--text-base);
    line-height: var(--leading);
    margin: 0;
  }
</style>
