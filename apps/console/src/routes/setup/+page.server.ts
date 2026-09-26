import type { PageServerLoad } from './$types';
import { redirect } from '@sveltejs/kit';
import { getSystem } from '$lib/server/vkdg/client';

export const load: PageServerLoad = async () => {
  try {
    const system = await getSystem();
    if (system.connection_count > 0) {
      throw redirect(302, '/');
    }
  } catch (e) {
    if (e && typeof e === 'object' && 'status' in e) throw e;
    // Gateway unreachable — still show setup
  }
  return {};
};
