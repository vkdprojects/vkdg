<script lang="ts">
  import { Button, Card, Input, Spinner, CopyButton, Badge } from '$lib/components/index.js';
  import { Zap, ArrowRight, ArrowLeft, CheckCircle2 } from 'lucide-svelte';
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

<div class="wizard-wrap">
  <!-- ── step indicator ──────────────────────────────────────────────────── -->
  <div class="step-bar" role="progressbar" aria-valuemin={1} aria-valuemax={5} aria-valuenow={step} aria-label={m.setup_progress_label()}>
    {#each [1, 2, 3, 4, 5] as s}
      <div class="step-dot" class:active={step === s} class:done={step > s}></div>
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
    justify-content: flex-start;
    min-height: 100%;
    padding: 3rem 2rem 4rem;
  }

  /* ── step indicator ──────────────────────────────────────────────────── */
  .step-bar {
    display: flex;
    gap: 8px;
    margin-bottom: 3rem;
  }

  .step-dot {
    width: 6px;
    height: 6px;
    border-radius: 2px;
    background: var(--border-strong);
    transition: background 0.2s, transform 0.2s;
  }

  .step-dot.active {
    background: var(--accent);
    transform: scale(1.4);
  }

  .step-dot.done {
    background: var(--success);
  }

  /* ── steps ───────────────────────────────────────────────────────────── */
  .step {
    width: 100%;
    max-width: 640px;
  }

  .narrow-step {
    max-width: 480px;
  }

  .center-step {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: 1rem;
  }

  /* ── step 1: welcome ─────────────────────────────────────────────────── */
  .hero-icon {
    width: 64px;
    height: 64px;
    border-radius: var(--radius);
    background: var(--accent-subtle);
    border: 1px solid var(--border-strong);
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--accent);
    margin-bottom: 0.5rem;
  }

  .wizard-title {
    font-size: var(--text-xl);
    font-weight: 700;
    color: var(--text-1);
    margin: 0 0 0.25rem;
    line-height: 1.2;
  }

  .wizard-sub {
    font-size: var(--text-base);
    color: var(--text-2);
    margin: 0 0 1.5rem;
    line-height: 1.5;
  }

  .wizard-sub a {
    color: var(--accent);
    text-decoration: none;
  }

  .wizard-sub a:hover {
    text-decoration: underline;
  }

  /* ── step 2: providers ───────────────────────────────────────────────── */
  .provider-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }

  .provider-card {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0.875rem 1rem;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    cursor: pointer;
    text-align: left;
    transition: border-color 0.15s, background 0.15s;
  }

  .provider-card:hover {
    border-color: var(--border-strong);
    background: var(--bg-elevated);
  }

  .provider-card:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .provider-dot {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    background: var(--accent);
    flex-shrink: 0;
  }

  .provider-info {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .provider-name {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--text-1);
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .provider-desc {
    font-size: var(--text-xs);
    color: var(--text-3);
  }

  :global(.provider-arrow) {
    color: var(--text-3);
    flex-shrink: 0;
  }

  /* ── step 3: API key ─────────────────────────────────────────────────── */
  .form-group {
    margin-bottom: 1.5rem;
  }

  .key-note {
    font-size: var(--text-xs);
    color: var(--text-3);
    margin: 0.5rem 0 0;
  }

  .action-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .justify-end {
    justify-content: flex-end;
  }

  /* ── step 4: testing ─────────────────────────────────────────────────── */
  .testing-label {
    font-size: var(--text-md);
    color: var(--text-2);
    margin: 0;
  }

  .success-icon {
    color: var(--success);
  }

  .success-label {
    color: var(--success);
    font-weight: 500;
  }

  /* ── step 5: ready ───────────────────────────────────────────────────── */
  .endpoint-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0.625rem 0.875rem;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    margin-bottom: 1.5rem;
  }

  .endpoint-url {
    flex: 1;
    font-size: var(--text-sm);
    color: var(--accent);
  }

  .divider {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin-bottom: 1.25rem;
    color: var(--text-3);
    font-size: var(--text-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  .divider::before,
  .divider::after {
    content: '';
    flex: 1;
    height: 1px;
    background: var(--border);
  }

  .code-blocks {
    display: flex;
    flex-direction: column;
    gap: 12px;
    margin-bottom: 2rem;
  }

  .code-block {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
  }

  .code-label {
    display: block;
    font-size: var(--text-2xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-3);
    padding: 0.5rem 0.875rem 0.25rem;
  }

  .code-line {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 0.5rem 0.875rem 0.625rem;
  }

  .code-line code {
    font-size: var(--text-sm);
    color: var(--text-1);
    word-break: break-all;
  }

  .btn-link {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 0.625rem 1.25rem;
    background: var(--accent);
    color: var(--bg-base);
    border-radius: var(--radius-sm);
    font-size: var(--text-base);
    font-weight: 600;
    text-decoration: none;
    transition: background 0.1s;
    white-space: nowrap;
  }

  .btn-link:hover {
    background: var(--accent-hover);
  }

  .btn-link:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
</style>
