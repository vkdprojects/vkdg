<script lang="ts">
  import { ExternalLink } from 'lucide-svelte';
  import { m } from '$lib/paraglide/messages.js';
  import { Button, CopyButton, Spinner } from '$lib/components/index.js';

  interface Props {
    userCode: string;
    verificationUrl: string;
    secondsLeft: number;
    oncancel: () => void;
  }

  let { userCode, verificationUrl, secondsLeft, oncancel }: Props = $props();

  const countdown = $derived(`${Math.floor(secondsLeft / 60)}:${String(secondsLeft % 60).padStart(2, '0')}`);
</script>

<p class="step-intro">{m.acct_device_step()}</p>
<div class="code-box">
  <span class="code-eyebrow mono">{m.acct_copy_code()}</span>
  <div class="code-row">
    <span class="user-code mono">{userCode}</span>
    <CopyButton text={userCode} label={m.acct_copy_code()} />
  </div>
</div>
<a class="verify-link" href={verificationUrl} target="_blank" rel="noopener noreferrer">
  <ExternalLink size={14} aria-hidden="true" />
  {m.acct_open_verification()}
</a>
<div class="muted status-row" role="status">
  <Spinner size="sm" /> {m.acct_waiting()}
</div>
<!-- Countdown updates every second; keep it out of the live region to avoid chatter. -->
<p class="muted mono status-row" aria-live="off">{m.acct_expires_in({ time: countdown })}</p>
<div class="dialog-footer">
  <Button variant="outline" onclick={oncancel}>{m.common_cancel()}</Button>
</div>

<style>
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
</style>
