<script lang="ts">
  import { EmptyState } from '$lib/components/index.js';
  import { m } from '$lib/paraglide/messages.js';
</script>

<div class="page">
  <div class="page-header">
    <h1 class="page-title">{m.settings_heading()}</h1>
  </div>

  <EmptyState title={m.settings_coming_soon()} />
</div>

<style>
  .page-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 1.5rem;
  }

  .page-title {
    margin: 0;
  }
</style>
