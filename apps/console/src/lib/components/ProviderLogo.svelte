<script lang="ts">
  /**
   * Real provider mark from static/providers/<id>.svg (LobeHub, MIT).
   * Monochrome marks are drawn with a CSS mask so they follow the theme's
   * text colour; colour marks render as-is. Unknown ids fall back to the
   * provider's own icon_char/icon_color from the admin API.
   */
  interface Props {
    id: string;
    name?: string;
    fallbackChar?: string;
    fallbackColor?: string;
    size?: 'sm' | 'md' | 'lg';
  }
  let { id, name = id, fallbackChar, fallbackColor, size = 'md' }: Props = $props();

  const MONO = new Set(['anthropic', 'cerebras', 'github-copilot', 'groq', 'openai']);
  const COLOR = new Set(['claude-code', 'antigravity', 'codex', 'deepseek', 'fireworks', 'gemini', 'kimi-coding', 'kiro', 'mistral', 'nvidia-nim', 'sambanova', 'together']);

  const kind = $derived(MONO.has(id) ? 'mono' : COLOR.has(id) ? 'color' : 'fallback');
  const src = $derived(`/providers/${id}.svg`);
</script>

<span class="logo {size}" data-kind={kind} title={name} style:--fallback={fallbackColor}>
  {#if kind === 'color'}
    <img {src} alt="" />
  {:else if kind === 'mono'}
    <span class="mask" style:--src={`url(${src})`}></span>
  {:else}
    <span class="char">{fallbackChar ?? name.slice(0, 1).toUpperCase()}</span>
  {/if}
  <span class="sr-only">{name}</span>
</span>

<style>
  .logo {
    --box: var(--control-h-lg);
    display: inline-grid; place-items: center; flex-shrink: 0;
    width: var(--box); height: var(--box);
    border-radius: var(--radius);
    background: var(--glass-bg-strong);
    border: var(--border-w) solid var(--glass-border);
    box-shadow: var(--glass-highlight);
  }
  .sm { --box: var(--control-h-sm); border-radius: var(--radius-sm); }
  .lg { --box: var(--space-8); border-radius: var(--radius-lg); }
  img, .mask { width: 58%; height: 58%; }
  img { object-fit: contain; }
  .mask {
    background: var(--text-1);
    -webkit-mask: var(--src) center / contain no-repeat;
    mask: var(--src) center / contain no-repeat;
  }
  .logo[data-kind='fallback'] { background: var(--fallback, var(--accent)); border-color: transparent; }
  .char { color: var(--on-accent); font-weight: var(--weight-bold); font-size: calc(var(--box) * 0.42); }
</style>
