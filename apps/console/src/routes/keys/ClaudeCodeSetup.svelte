<script lang="ts">
  import { m } from '$lib/paraglide/messages.js';
</script>

<section aria-labelledby="claude-heading" class="panel">
  <div class="panel-head">
    <h2 id="claude-heading">{m.key_claude_code_heading()}</h2>
  </div>
  <div class="panel-body">
    <p class="instructions">
      Set <code>ANTHROPIC_BASE_URL=http://localhost:8080</code> and
      <code>ANTHROPIC_API_KEY=&lt;your-key&gt;</code> in your Claude Code config.
    </p>
  </div>
</section>

<style>
  .instructions {
    font-size: var(--text-base);
    color: var(--text-2);
    line-height: var(--leading);
    margin: 0;
  }

  .instructions code {
    font-size: var(--text-sm);
    color: var(--text-1);
  }
</style>
