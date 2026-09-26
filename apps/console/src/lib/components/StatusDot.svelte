<script lang="ts">
  type Status = 'healthy' | 'degraded' | 'circuit_open' | 'cooldown' | string;

  interface Props {
    status: Status;
    size?: number;
  }

  let { status, size = 8 }: Props = $props();

  const colorMap: Record<string, string> = {
    healthy: 'var(--success)',
    degraded: 'var(--warning)',
    circuit_open: 'var(--danger)',
    cooldown: 'oklch(0.62 0.18 290)',
  };

  const color = $derived(colorMap[status] ?? 'var(--text-3)');
  const pulse = $derived(status === 'healthy');
</script>

<span
  class="dot"
  class:pulse
  style="--dot-color: {color}; --dot-size: {size}px"
  title={status}
></span>

<style>
  .dot {
    display: inline-block;
    width: var(--dot-size);
    height: var(--dot-size);
    border-radius: 50%;
    background: var(--dot-color);
    flex-shrink: 0;
  }

  @keyframes pulse {
    0%, 100% { box-shadow: 0 0 0 0 color-mix(in oklch, var(--dot-color) 50%, transparent); }
    50% { box-shadow: 0 0 0 4px color-mix(in oklch, var(--dot-color) 0%, transparent); }
  }

  .pulse {
    animation: pulse 2.5s ease-in-out infinite;
  }
</style>
