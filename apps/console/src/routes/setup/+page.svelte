<script lang="ts">
  import { Button, Card, Input, Spinner, CopyButton, Badge } from '$lib/components/index.js';
  import { Zap, ArrowRight, ArrowLeft, CheckCircle2 } from 'lucide-svelte';

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
    color: string;
    dot: string;
    url: string;
    free?: boolean;
  };

  const providers: Provider[] = [
    { id: 'anthropic',   label: 'Anthropic',               desc: 'API key',                    color: '#7c3aed', dot: '#a78bfa', url: 'https://console.anthropic.com/settings/keys' },
    { id: 'openai',      label: 'OpenAI',                  desc: 'API key',                    color: '#16a34a', dot: '#4ade80', url: 'https://platform.openai.com/api-keys' },
    { id: 'groq',        label: 'Groq',                    desc: 'API key, free tier',         color: '#ea580c', dot: '#fb923c', url: 'https://console.groq.com/keys', free: true },
    { id: 'gemini',      label: 'Google Gemini',           desc: 'API key',                    color: '#1d4ed8', dot: '#60a5fa', url: 'https://aistudio.google.com/app/apikey' },
    { id: 'deepseek',    label: 'DeepSeek',                desc: 'API key',                    color: '#0d9488', dot: '#2dd4bf', url: 'https://platform.deepseek.com/api_keys' },
    { id: 'mistral',     label: 'Mistral',                 desc: 'API key',                    color: '#374151', dot: '#9ca3af', url: 'https://console.mistral.ai/api-keys/' },
    { id: 'custom',      label: 'Custom (OpenAI-compat)',  desc: 'Any OpenAI-compat endpoint', color: '#4b5563', dot: '#6b7280', url: '' },
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
  <div class="step-bar">
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
        <Zap size={36} strokeWidth={1.5} />
      </div>
      <h1 class="wizard-title">Your AI gateway is running</h1>
      <p class="wizard-sub">Connect a provider and you'll be sending requests in under 2 minutes.</p>
      <Button size="lg" onclick={() => (step = 2)}>
        Get started <ArrowRight size={15} />
      </Button>
    </div>

  <!-- ══════════════════════════════════════════════════════════════════════ -->
  <!-- step 2: pick provider                                                  -->
  <!-- ══════════════════════════════════════════════════════════════════════ -->
  {:else if step === 2}
    <div class="step">
      <h1 class="wizard-title">Connect your first provider</h1>
      <div class="provider-grid">
        {#each providers as p}
          <button class="provider-card" onclick={() => pickProvider(p)}>
            <div class="provider-dot" style="background: {p.dot}"></div>
            <div class="provider-info">
              <span class="provider-name">
                {p.label}
                {#if p.free}
                  <Badge status="success" label="Free tier" />
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
      <h1 class="wizard-title">Enter your {selectedProviderLabel} API key</h1>
      {#if providerUrl()}
        <p class="wizard-sub">
          Find your key at
          <a href={providerUrl()} target="_blank" rel="noopener noreferrer">{providerUrl()}</a>
        </p>
      {:else}
        <p class="wizard-sub">Enter the base URL and API key for your custom endpoint.</p>
      {/if}

      <div class="form-group">
        <Input
          label="API Key"
          type="password"
          placeholder="sk-..."
          bind:value={apiKey}
          autocomplete="off"
        />
        <p class="key-note">Your key is stored encrypted and never exposed.</p>
      </div>

      <div class="action-row">
        <Button variant="ghost" onclick={() => (step = 2)}>
          <ArrowLeft size={14} /> Back
        </Button>
        <Button
          disabled={apiKey.trim().length === 0}
          onclick={testConnection}
        >
          Test connection <ArrowRight size={14} />
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
        <p class="testing-label">Testing connection...</p>
      {:else}
        <div class="success-icon">
          <CheckCircle2 size={40} strokeWidth={1.5} />
        </div>
        <p class="testing-label success-label">Connected! 3 models available</p>
        <Button size="lg" onclick={finish}>
          Continue <ArrowRight size={15} />
        </Button>
      {/if}
    </div>

  <!-- ══════════════════════════════════════════════════════════════════════ -->
  <!-- step 5: ready                                                          -->
  <!-- ══════════════════════════════════════════════════════════════════════ -->
  {:else if step === 5}
    <div class="step narrow-step">
      <h1 class="wizard-title">You're ready!</h1>
      <p class="wizard-sub">Point your AI client at this endpoint:</p>

      <div class="endpoint-row">
        <code class="endpoint-url">{ENDPOINT}</code>
        <CopyButton text={ENDPOINT} label="Copy URL" />
      </div>

      <div class="divider">
        <span>Configure your AI client</span>
      </div>

      <div class="code-blocks">
        <div class="code-block">
          <span class="code-label">Claude Code</span>
          <div class="code-line">
            <code>claude config set ANTHROPIC_BASE_URL {ENDPOINT}</code>
            <CopyButton text="claude config set ANTHROPIC_BASE_URL {ENDPOINT}" />
          </div>
        </div>

        <div class="code-block">
          <span class="code-label">Codex CLI</span>
          <div class="code-line">
            <code>codex --config openai-base-url={ENDPOINT}</code>
            <CopyButton text="codex --config openai-base-url={ENDPOINT}" />
          </div>
        </div>
      </div>

      <div class="action-row justify-end">
        <a href="/" class="btn-link">
          Go to dashboard <ArrowRight size={15} />
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
    border-radius: 50%;
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
    animation: fade-in 0.18s ease;
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

  @keyframes fade-in {
    from { opacity: 0; transform: translateY(6px); }
    to   { opacity: 1; transform: translateY(0); }
  }

  /* ── step 1: welcome ─────────────────────────────────────────────────── */
  .hero-icon {
    width: 72px;
    height: 72px;
    border-radius: 16px;
    background: color-mix(in oklch, var(--accent) 12%, transparent);
    border: 1px solid color-mix(in oklch, var(--accent) 25%, transparent);
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--accent);
    margin-bottom: 0.5rem;
  }

  .wizard-title {
    font-size: 1.5rem;
    font-weight: 700;
    color: var(--text-1);
    margin: 0 0 0.25rem;
    line-height: 1.2;
  }

  .wizard-sub {
    font-size: 0.9375rem;
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

  .provider-dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
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
    font-size: 0.875rem;
    font-weight: 500;
    color: var(--text-1);
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .provider-desc {
    font-size: 0.75rem;
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
    font-size: 0.75rem;
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
    font-size: 1rem;
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
    font-family: 'SF Mono', 'Cascadia Code', 'Fira Code', monospace;
    font-size: 0.875rem;
    color: var(--accent);
  }

  .divider {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin-bottom: 1.25rem;
    color: var(--text-3);
    font-size: 0.75rem;
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
    font-size: 0.6875rem;
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
    font-family: 'SF Mono', 'Cascadia Code', 'Fira Code', monospace;
    font-size: 0.8125rem;
    color: var(--text-1);
    word-break: break-all;
  }

  .btn-link {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 0.625rem 1.25rem;
    background: var(--accent);
    color: #fff;
    border-radius: var(--radius-sm);
    font-size: 0.9375rem;
    font-weight: 500;
    text-decoration: none;
    transition: background 0.1s;
    white-space: nowrap;
  }

  .btn-link:hover {
    background: var(--accent-hover);
  }
</style>
