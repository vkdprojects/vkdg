import type { PageServerLoad, Actions } from './$types';
import { listKeys, createKey, revokeKey } from '$lib/server/vkdg/client';
import { redirect } from '@sveltejs/kit';

export const load: PageServerLoad = async ({ request }) => {
  const cookie = request.headers.get('cookie') ?? '';
  const keys = await listKeys(cookie);
  return { keys };
};

export const actions: Actions = {
  create: async ({ request }) => {
    const cookie = request.headers.get('cookie') ?? '';
    const form = await request.formData();
    const name = form.get('name')?.toString() ?? '';
    const role = form.get('role')?.toString() ?? 'viewer';
    if (!name.trim()) return { error: 'Name is required' };
    try {
      const created = await createKey(cookie, name, role);
      return { created };
    } catch (e) {
      return { error: (e as Error).message };
    }
  },
  revoke: async ({ request }) => {
    const cookie = request.headers.get('cookie') ?? '';
    const form = await request.formData();
    const id = form.get('id')?.toString() ?? '';
    await revokeKey(cookie, id).catch(() => {});
    redirect(302, '/keys');
  },
};
