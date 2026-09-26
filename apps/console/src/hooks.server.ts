import type { Handle } from '@sveltejs/kit';
import { getMe } from '$lib/server/vkdg/client';

export const handle: Handle = async ({ event, resolve }) => {
  const cookie = event.request.headers.get('cookie') ?? '';
  const sessionCookie = cookie
    .split(';')
    .find((c) => c.trim().startsWith('vkdg_session='));

  if (sessionCookie) {
    try {
      event.locals.user = await getMe(cookie);
    } catch {
      event.locals.user = null;
    }
  } else {
    event.locals.user = null;
  }

  const path = event.url.pathname;
  if (!event.locals.user && path !== '/login') {
    return new Response(null, { status: 302, headers: { Location: '/login' } });
  }

  return resolve(event);
};
