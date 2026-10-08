<script lang="ts">
  import { EmptyState } from '$lib/components/index.js';
  import { m } from '$lib/paraglide/messages.js';
</script>

<div class="page">
  <div class="page-header">
    <div>
      <h1>{m.settings_heading()}</h1>
    </div>
  </div>

  <section class="panel">
    <div class="panel-body">
      <EmptyState title={m.settings_coming_soon()} />
    </div>
  </section>
</div>
