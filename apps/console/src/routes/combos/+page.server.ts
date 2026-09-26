import type { PageServerLoad, Actions } from './$types';
import { listCombos } from '$lib/server/vkdg/client';

export const load: PageServerLoad = async ({ request }) => {
  const cookie = request.headers.get('cookie') ?? '';
  const combos = await listCombos(cookie).catch(() => []);
  return { combos };
};

export const actions: Actions = {
  create: async () => {
    return { error: 'Not yet implemented' };
  },
};
