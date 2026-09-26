import type { PageServerLoad } from './$types';
import { listConnections } from '$lib/server/vkdg/client';

export const load: PageServerLoad = async ({ request }) => {
  const cookie = request.headers.get('cookie') ?? '';
  const connections = await listConnections(cookie);
  return { connections };
};

export const actions = {
  add: async () => {
    // Phase E: admin API connection creation
    return { success: false, error: 'Connection creation via UI is coming in the next release. Use vkdg.yaml for now.' };
  }
};
