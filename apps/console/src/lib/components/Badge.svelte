<script lang="ts">
  type Status = 'healthy' | 'degraded' | 'circuit_open' | 'cooldown' | string;

  interface Props {
    status: Status;
    label?: string;
  }

  let { status, label }: Props = $props();

  const colorMap: Record<string, string> = {
    healthy: 'var(--success)',
    degraded: 'var(--warning)',
    circuit_open: 'var(--danger)',
    cooldown: 'var(--cooldown)',
    success: 'var(--success)',
    error: 'var(--danger)',
    pending: 'var(--warning)',
    completed: 'var(--success)',
    partial: 'var(--warning)',
    cancelled: 'var(--text-3)',
    failed: 'var(--danger)',
  };

  const color = $derived(colorMap[status] ?? 'var(--text-3)');
  const text = $derived(label ?? status.replace(/_/g, ' '));
</script>

<span class="badge" style="--badge-color: {color}">
  {text}
</span>

<style>
  .badge {
    display: inline-flex; align-items: center; gap: var(--space-1);
    padding: var(--space-0) var(--space-2) var(--space-0) var(--space-1);
    border-radius: var(--radius-full);
    font-size: var(--text-xs); font-weight: var(--weight-medium);
    background: color-mix(in oklch, var(--badge-color) 14%, transparent);
    color: var(--badge-color);
    border: 1px solid color-mix(in oklch, var(--badge-color) 28%, transparent);
    white-space: nowrap;
  }
  .badge::first-letter { text-transform: uppercase; }
  .badge::before { content: ''; width: var(--dot-size); height: var(--dot-size); border-radius: var(--radius-full); background: currentColor; }
</style>
