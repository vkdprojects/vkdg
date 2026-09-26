import type { PageServerLoad } from './$types';
import { listConnections } from '$lib/server/vkdg/client';

export const load: PageServerLoad = async ({ request }) => {
  const cookie = request.headers.get('cookie') ?? '';
  try {
    const connections = await listConnections(cookie);
    return { connections };
  } catch {
    return { connections: [] };
  }
};
