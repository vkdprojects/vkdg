<script lang="ts">
  import { AlertDialog } from 'bits-ui';
  import { m } from '$lib/paraglide/messages.js';

  interface Props {
    open: boolean;
    title: string;
    description: string;
    confirmLabel: string;
    busy?: boolean;
    onConfirm: () => void | Promise<void>;
    /** Fires when the dialog is dismissed (Esc, overlay, Cancel); lets owners drop their pending target. */
    onClose?: () => void;
  }

  let { open = $bindable(false), title, description, confirmLabel, busy = false, onConfirm, onClose }: Props = $props();
</script>

<AlertDialog.Root bind:open onOpenChange={(v) => { if (!v) onClose?.(); }}>
  <AlertDialog.Portal>
    <AlertDialog.Overlay class="dialog-overlay" />
    <AlertDialog.Content class="dialog-content">
      <div class="dialog-header">
        <AlertDialog.Title class="dialog-title">{title}</AlertDialog.Title>
      </div>
      <AlertDialog.Description class="dialog-body desc">{description}</AlertDialog.Description>
      <div class="dialog-footer">
        <AlertDialog.Cancel class="btn-like outline">{m.common_cancel()}</AlertDialog.Cancel>
        <button type="button" class="btn-like danger" disabled={busy} onclick={onConfirm}>{confirmLabel}</button>
      </div>
    </AlertDialog.Content>
  </AlertDialog.Portal>
</AlertDialog.Root>

<style>
  /* Portaled content sits outside this component's DOM scope, hence :global. */
  :global(.dialog-body.desc) { color: var(--text-2); font-size: var(--text-sm); margin: 0; }
  :global(.btn-like) {
    display: inline-flex; align-items: center; justify-content: center;
    height: var(--control-h); padding: 0 var(--control-px);
    border: var(--border-w) solid var(--border-strong); border-radius: var(--radius);
    background: var(--bg-surface); color: var(--text-1); font: inherit; font-size: var(--text-sm); font-weight: var(--weight-medium);
    cursor: pointer;
  }
  :global(.btn-like:hover) { background: var(--bg-hover); }
  :global(.btn-like.danger) { background: var(--danger); border-color: var(--danger); color: var(--on-accent); }
  :global(.btn-like.danger:hover) { background: color-mix(in oklch, var(--danger) 88%, var(--text-1)); }
  :global(.btn-like:disabled) { opacity: 0.45; cursor: not-allowed; }
</style>
