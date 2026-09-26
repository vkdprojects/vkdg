import type { PageServerLoad } from './$types';
import { listCombos } from '$lib/server/vkdg/client';

export const load: PageServerLoad = async ({ request }) => {
  const cookie = request.headers.get('cookie') ?? '';
  const combos = await listCombos(cookie).catch(() => []);
  return { combos };
};
