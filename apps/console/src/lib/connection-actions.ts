import { toast } from 'svelte-sonner';
import { api } from '$lib/api.js';
import { m } from '$lib/paraglide/messages.js';

/**
 * Connection mutations shared by the connections and provider-detail screens:
 * call the API, toast the outcome, and tell the caller whether it succeeded so
 * it can reload and clear its own busy state.
 */

export async function deleteConnection(id: string): Promise<boolean> {
  try {
    await api.deleteConnection(id);
    toast.success(m.connection_deleted());
    return true;
  } catch (e) {
    toast.error((e as Error).message);
    return false;
  }
}

export async function syncConnectionModels(id: string): Promise<boolean> {
  try {
    const result = await api.syncConnectionModels(id);
    toast.success(m.connection_models_synced({ count: result.count }));
    return true;
  } catch (e) {
    toast.error((e as Error).message);
    return false;
  }
}
