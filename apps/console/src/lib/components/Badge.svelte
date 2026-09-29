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
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 0.125rem 0.5rem;
    border-radius: var(--radius-sm);
    font-size: var(--text-2xs);
    font-weight: 600;
    font-family: var(--font-mono);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    background: color-mix(in oklch, var(--badge-color) 14%, transparent);
    color: var(--badge-color);
    border: 1px solid color-mix(in oklch, var(--badge-color) 34%, transparent);
    white-space: nowrap;
  }
</style>
