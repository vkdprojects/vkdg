import type { LayoutServerLoad } from './$types';
import { getSystem } from '$lib/server/vkdg/client';

export const load: LayoutServerLoad = async ({ locals }) => {
  let system = null;
  try {
    system = await getSystem();
  } catch {}
  return { user: locals.user ?? null, system };
};
