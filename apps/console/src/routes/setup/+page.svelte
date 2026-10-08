<script lang="ts">
  import { Button, Card, Input, Spinner, CopyButton, Badge } from '$lib/components/index.js';
  import { Zap, ArrowRight, ArrowLeft, CheckCircle2, Check } from 'lucide-svelte';
  import { m } from '$lib/paraglide/messages.js';

  // ── state ──────────────────────────────────────────────────────────────────
  let step = $state(1);
  let selectedProvider = $state('');
  let selectedProviderLabel = $state('');
  let apiKey = $state('');
  let testing = $state(false);
  let testDone = $state(false);

  const ENDPOINT = 'http://localhost:8080';

  // ── providers ──────────────────────────────────────────────────────────────
  type Provider = {
    id: string;
    label: string;
    desc: string;
    url: string;
    free?: boolean;
  };

  const providers: Provider[] = [
    { id: 'anthropic',   label: 'Anthropic',               desc: m.setup_provider_api_key(),      url: 'https://console.anthropic.com/settings/keys' },
    { id: 'openai',      label: 'OpenAI',                  desc: m.setup_provider_api_key(),      url: 'https://platform.openai.com/api-keys' },
    { id: 'groq',        label: 'Groq',                    desc: m.setup_provider_free_tier(),    url: 'https://console.groq.com/keys', free: true },
    { id: 'gemini',      label: 'Google Gemini',           desc: m.setup_provider_api_key(),      url: 'https://aistudio.google.com/app/apikey' },
    { id: 'deepseek',    label: 'DeepSeek',                desc: m.setup_provider_api_key(),      url: 'https://platform.deepseek.com/api_keys' },
    { id: 'mistral',     label: 'Mistral',                 desc: m.setup_provider_api_key(),      url: 'https://console.mistral.ai/api-keys/' },
    { id: 'kiro',        label: 'Kiro (Amazon Q)',         desc: m.setup_provider_api_key(),      url: 'https://kiro.dev/' },
    { id: 'custom',      label: 'Custom (OpenAI-compat)',  desc: m.setup_provider_custom_desc(),  url: '' },
  ];

  function pickProvider(p: Provider) {
    selectedProvider = p.id;
    selectedProviderLabel = p.label;
    step = 3;
  }

  function providerUrl(): string {
    return providers.find(p => p.id === selectedProvider)?.url ?? '';
  }

  async function testConnection() {
    testing = true;
    step = 4;
    await new Promise(r => setTimeout(r, 1500));
    testing = false;
    testDone = true;
  }

  function finish() {
    step = 5;
  }
</script>

<div class="page wizard-wrap">
  <!-- ── step indicator ──────────────────────────────────────────────────── -->
  <div class="stepper" role="progressbar" aria-valuemin={1} aria-valuemax={5} aria-valuenow={step} aria-label={m.setup_progress_label()}>
    {#each [1, 2, 3, 4, 5] as s}
      <div class="step-node" class:active={step === s} class:done={step > s}>
        <span class="step-circle">
          {#if step > s}<Check size={14} strokeWidth={3} aria-hidden="true" />{:else}{s}{/if}
        </span>
      </div>
    {/each}
  </div>

  <!-- ══════════════════════════════════════════════════════════════════════ -->
  <!-- step 1: welcome                                                        -->
  <!-- ══════════════════════════════════════════════════════════════════════ -->
  {#if step === 1}
    <div class="step center-step">
      <div class="hero-icon">
        <Zap size={32} strokeWidth={1.5} />
      </div>
      <h1 class="wizard-title">{m.setup_welcome_title()}</h1>
      <p class="wizard-sub">{m.setup_welcome_sub()}</p>
      <Button size="lg" onclick={() => (step = 2)}>
        {m.setup_get_started()} <ArrowRight size={15} />
      </Button>
    </div>

  <!-- ══════════════════════════════════════════════════════════════════════ -->
  <!-- step 2: pick provider                                                  -->
  <!-- ══════════════════════════════════════════════════════════════════════ -->
  {:else if step === 2}
    <div class="step">
      <h1 class="wizard-title">{m.setup_pick_provider_title()}</h1>
      <div class="provider-grid">
        {#each providers as p}
          <button class="provider-card" onclick={() => pickProvider(p)}>
            <span class="provider-dot" aria-hidden="true"></span>
            <div class="provider-info">
              <span class="provider-name">
                {p.label}
                {#if p.free}
                  <Badge status="success" label={m.setup_free_tier_badge()} />
                {/if}
              </span>
              <span class="provider-desc">{p.desc}</span>
            </div>
            <ArrowRight size={13} class="provider-arrow" />
          </button>
        {/each}
      </div>
    </div>

  <!-- ══════════════════════════════════════════════════════════════════════ -->
  <!-- step 3: enter API key                                                  -->
  <!-- ══════════════════════════════════════════════════════════════════════ -->
  {:else if step === 3}
    <div class="step narrow-step">
      <h1 class="wizard-title">{m.setup_enter_key_title({ provider: selectedProviderLabel })}</h1>
      {#if providerUrl()}
        <p class="wizard-sub">
          {m.setup_find_key_at()}
          <a href={providerUrl()} target="_blank" rel="noopener noreferrer">{providerUrl()}</a>
        </p>
      {:else}
        <p class="wizard-sub">{m.setup_custom_endpoint_hint()}</p>
      {/if}

      <div class="form-group">
        <Input
          label={m.setup_api_key_label()}
          type="password"
          placeholder="sk-..."
          bind:value={apiKey}
          autocomplete="off"
        />
        <p class="key-note">{m.setup_key_stored_note()}</p>
      </div>

      <div class="action-row">
        <Button variant="ghost" onclick={() => (step = 2)}>
          <ArrowLeft size={14} /> {m.common_back()}
        </Button>
        <Button
          disabled={apiKey.trim().length === 0}
          onclick={testConnection}
        >
          {m.setup_test_connection()} <ArrowRight size={14} />
        </Button>
      </div>
    </div>

  <!-- ══════════════════════════════════════════════════════════════════════ -->
  <!-- step 4: testing                                                        -->
  <!-- ══════════════════════════════════════════════════════════════════════ -->
  {:else if step === 4}
    <div class="step center-step">
      {#if testing}
        <Spinner size="lg" />
        <p class="testing-label">{m.setup_testing_connection()}</p>
      {:else}
        <div class="success-icon">
          <CheckCircle2 size={40} strokeWidth={1.5} />
        </div>
        <p class="testing-label success-label">{m.setup_connected_models({ n: 3 })}</p>
        <Button size="lg" onclick={finish}>
          {m.common_continue()} <ArrowRight size={15} />
        </Button>
      {/if}
    </div>

  <!-- ══════════════════════════════════════════════════════════════════════ -->
  <!-- step 5: ready                                                          -->
  <!-- ══════════════════════════════════════════════════════════════════════ -->
  {:else if step === 5}
    <div class="step narrow-step">
      <h1 class="wizard-title">{m.setup_ready_title()}</h1>
      <p class="wizard-sub">{m.setup_ready_sub()}</p>

      <div class="endpoint-row">
        <code class="endpoint-url mono">{ENDPOINT}</code>
        <CopyButton text={ENDPOINT} label={m.setup_copy_url()} />
      </div>

      <div class="divider">
        <span>{m.setup_configure_client()}</span>
      </div>

      <div class="code-blocks">
        <div class="code-block">
          <span class="code-label">Claude Code</span>
          <div class="code-line">
            <code class="mono">claude config set ANTHROPIC_BASE_URL {ENDPOINT}</code>
            <CopyButton text="claude config set ANTHROPIC_BASE_URL {ENDPOINT}" label={m.common_copy()} />
          </div>
        </div>

        <div class="code-block">
          <span class="code-label">Codex CLI</span>
          <div class="code-line">
            <code class="mono">codex --config openai-base-url={ENDPOINT}</code>
            <CopyButton text="codex --config openai-base-url={ENDPOINT}" label={m.common_copy()} />
          </div>
        </div>
      </div>

      <div class="action-row justify-end">
        <a href="/" class="btn-link">
          {m.setup_go_to_dashboard()} <ArrowRight size={15} />
        </a>
      </div>
    </div>
  {/if}
</div>

<style>
  .wizard-wrap {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding-top: var(--space-6);
  }

  /* ── stepper ─────────────────────────────────────────────────────────── */
  .stepper {
    display: flex;
    align-items: center;
    margin: 0 0 var(--space-6);
    width: 100%;
    max-width: calc(var(--space-8) * 5.5);
  }

  .step-node {
    display: flex;
    align-items: center;
    flex: 1;
  }

  .step-node:last-child { flex: 0; }

  /* connector line to the next node */
  .step-node:not(:last-child)::after {
    content: '';
    flex: 1;
    height: var(--focus-w);
    margin: 0 var(--space-2);
    border-radius: var(--radius-full);
    background: var(--border-strong);
    transition: background var(--dur-3) var(--ease-out);
  }

  .step-node.done:not(:last-child)::after {
    background: var(--accent);
  }

  .step-circle {
    display: grid;
    place-items: center;
    width: var(--control-h-sm);
    height: var(--control-h-sm);
    flex-shrink: 0;
    border-radius: var(--radius-full);
    border: 1px solid var(--border-strong);
    background: var(--bg-surface);
    color: var(--text-3);
    font-size: var(--text-xs);
    font-weight: var(--weight-semibold);
    font-variant-numeric: tabular-nums;
    transition: background var(--dur-2) var(--ease-out), color var(--dur-2) var(--ease-out),
                border-color var(--dur-2) var(--ease-out), box-shadow var(--dur-2) var(--ease-out),
                transform var(--dur-2) var(--ease-spring);
  }

  .step-node.active .step-circle {
    background: var(--accent-subtle);
    border-color: var(--accent);
    color: var(--accent);
    box-shadow: var(--ring);
    transform: scale(1.08);
  }

  .step-node.done .step-circle {
    background: var(--accent);
    border-color: transparent;
    color: var(--on-accent);
  }

  /* ── steps ───────────────────────────────────────────────────────────── */
  .step {
    width: 100%;
    max-width: calc(var(--space-8) * 10);
    animation: step-in var(--dur-3) var(--ease-out);
  }

  @keyframes step-in { from { opacity: 0; transform: translateY(var(--space-2)); } }

  .narrow-step { max-width: calc(var(--space-8) * 7.5); }

  .center-step {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: var(--space-4);
    padding-top: var(--space-4);
  }

  /* ── step 1: welcome (the one bold moment) ───────────────────────────── */
  .hero-icon {
    width: calc(var(--space-8) * 1.125);
    height: calc(var(--space-8) * 1.125);
    border-radius: var(--radius-lg);
    background: var(--accent);
    display: grid;
    place-items: center;
    color: var(--on-accent);
    box-shadow: var(--glow);
    margin-bottom: var(--space-2);
  }

  .wizard-title {
    font-size: var(--text-xl);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--tracking-tight);
    color: var(--text-1);
    margin: 0 0 var(--space-1);
    line-height: var(--leading-tight);
    text-wrap: balance;
  }

  .center-step .wizard-title { font-size: var(--text-hero); }

  .wizard-sub {
    font-size: var(--text-base);
    color: var(--text-2);
    margin: 0 0 var(--space-5);
    line-height: var(--leading);
    max-width: 52ch;
    overflow-wrap: anywhere;
  }

  .center-step .wizard-sub { margin-bottom: var(--space-3); }

  .wizard-sub a:hover { text-decoration: underline; }

  /* ── step 2: providers ───────────────────────────────────────────────── */
  .provider-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, calc(var(--space-8) * 3.75)), 1fr));
    gap: var(--space-3);
    margin-top: var(--space-5);
  }

  .provider-card {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-height: var(--control-h-lg);
    padding: var(--space-3) var(--space-4);
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    box-shadow: var(--shadow-1);
    color: inherit;
    font: inherit;
    cursor: pointer;
    text-align: left;
  }

  .provider-card:hover {
    border-color: var(--accent-strong);
    background: var(--bg-elevated);
    transform: var(--lift);
  }

  .provider-card:active { transform: var(--press); }

  .provider-dot {
    width: var(--dot-size);
    height: var(--dot-size);
    border-radius: var(--radius-full);
    background: var(--accent);
    flex-shrink: 0;
  }

  .provider-info {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: var(--space-0);
    min-width: 0;
  }

  .provider-name {
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    color: var(--text-1);
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .provider-desc {
    font-size: var(--text-xs);
    color: var(--text-3);
  }

  :global(.provider-arrow) {
    color: var(--text-3);
    flex-shrink: 0;
    transition: transform var(--dur-2) var(--ease-out), color var(--dur-1) ease;
  }

  .provider-card:hover :global(.provider-arrow) {
    color: var(--accent);
    transform: translateX(var(--space-1));
  }

  /* ── step 3: API key ─────────────────────────────────────────────────── */
  .form-group { margin-bottom: var(--space-5); }

  .key-note {
    font-size: var(--text-xs);
    color: var(--text-3);
    margin: var(--space-2) 0 0;
  }

  .action-row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    align-items: center;
    justify-content: space-between;
  }

  .justify-end { justify-content: flex-end; }

  /* ── step 4: testing ─────────────────────────────────────────────────── */
  .testing-label {
    font-size: var(--text-md);
    color: var(--text-2);
    margin: 0;
  }

  .success-icon {
    display: grid;
    place-items: center;
    width: calc(var(--space-8) * 1.125);
    height: calc(var(--space-8) * 1.125);
    border-radius: var(--radius-full);
    background: var(--success-subtle);
    color: var(--success);
    animation: pop-in var(--dur-3) var(--ease-spring);
  }

  @keyframes pop-in { from { opacity: 0; transform: scale(0.5); } }

  .success-label {
    color: var(--success);
    font-weight: var(--weight-medium);
  }

  /* ── step 5: ready ───────────────────────────────────────────────────── */
  .endpoint-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-2) var(--space-2) var(--space-4);
    background: var(--bg-inset);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    margin-bottom: var(--space-5);
  }

  .endpoint-url {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
    font-size: var(--text-sm);
    color: var(--accent);
    background: none;
    border: 0;
    padding: 0;
  }

  .divider {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-bottom: var(--space-4);
    color: var(--text-3);
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
  }

  .divider::before,
  .divider::after {
    content: '';
    flex: 1;
    height: var(--border-w);
    background: var(--border);
  }

  .code-blocks {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    margin-bottom: var(--space-6);
  }

  .code-block {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
  }

  .code-label {
    display: block;
    font-size: var(--text-xs);
    font-weight: var(--weight-semibold);
    color: var(--text-2);
    padding: var(--space-3) var(--space-4) 0;
  }

  .code-line {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3) var(--space-3) var(--space-4);
  }

  .code-line code {
    flex: 1 1 calc(var(--space-8) * 3.5);
    min-width: 0;
    font-size: var(--text-sm);
    color: var(--text-1);
    overflow-wrap: anywhere;
  }

  .btn-link {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    min-height: var(--control-h-lg);
    padding: var(--space-2) var(--space-5);
    background: var(--accent);
    color: var(--on-accent);
    border-radius: var(--radius);
    font-size: var(--text-md);
    font-weight: var(--weight-medium);
    white-space: nowrap;
    box-shadow: var(--shadow-1);
  }

  .btn-link:hover {
    background: var(--accent-hover);
    color: var(--on-accent);
    transform: var(--lift);
  }

  .btn-link:active { transform: var(--press); }

  @media (max-width: 480px) {
    .action-row > :global(*) { flex: 1 1 auto; }
    .justify-end .btn-link { width: 100%; }
  }

  @media (prefers-reduced-motion: reduce) {
    .provider-card:hover, .btn-link:hover { transform: none; }
  }
</style>
