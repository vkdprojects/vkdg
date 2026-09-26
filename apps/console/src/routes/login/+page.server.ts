import type { Actions, PageServerLoad } from './$types';
import { redirect } from '@sveltejs/kit';
import { login } from '$lib/server/vkdg/client';

export const load: PageServerLoad = async ({ locals }) => {
  if (locals.user) throw redirect(302, '/');
  return {};
};

export const actions: Actions = {
  default: async ({ request, cookies }) => {
    const form = await request.formData();
    const token = form.get('token')?.toString() ?? '';
    try {
      const { setCookie } = await login(token);
      if (setCookie) {
        const [cookiePart] = setCookie.split(';');
        const eqIdx = cookiePart.indexOf('=');
        const value = cookiePart.slice(eqIdx + 1);
        cookies.set('vkdg_session', value, {
          httpOnly: true,
          sameSite: 'lax',
          path: '/',
          secure: false,
        });
      }
    } catch (e: unknown) {
      if (e instanceof Response) throw e;
      return { error: (e as Error).message };
    }
    throw redirect(302, '/');
  },
};
