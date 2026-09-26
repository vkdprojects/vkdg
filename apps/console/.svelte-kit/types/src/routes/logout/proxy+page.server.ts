// @ts-nocheck
import type { Actions } from './$types';
import { redirect } from '@sveltejs/kit';
import { logout } from '$lib/server/vkdg/client';

export const actions = {
  default: async ({ cookies, request }: import('./$types').RequestEvent) => {
    const cookie = request.headers.get('cookie') ?? '';
    await logout(cookie).catch(() => {});
    cookies.delete('vkdg_session', { path: '/' });
    throw redirect(302, '/login');
  },
};
;null as any as Actions;