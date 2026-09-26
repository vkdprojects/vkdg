<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { api } from '$lib/api.js';

  onMount(async () => {
    try {
      await api.logout();
    } catch { /* ignore */ }
    goto('/login');
  });
</script>
