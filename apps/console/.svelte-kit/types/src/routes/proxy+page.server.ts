// @ts-nocheck
import type { PageServerLoad } from './$types';
import { getSystem, listConnections } from '$lib/server/vkdg/client';

export const load = async ({ locals, request }: Parameters<PageServerLoad>[0]) => {
  const cookie = request.headers.get('cookie') ?? '';
  const [system, connections] = await Promise.all([
    getSystem(),
    listConnections(cookie),
  ]);
  return { user: locals.user, system, connections };
};
