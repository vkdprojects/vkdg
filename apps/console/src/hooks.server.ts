import type { Handle } from '@sveltejs/kit';
import { sequence } from '@sveltejs/kit/hooks';
import { paraglideMiddleware } from '$lib/paraglide/server';
import { getTextDirection } from '$lib/paraglide/runtime';
import { getMe } from '$lib/server/vkdg/client';

const i18n: Handle = ({ event, resolve }) =>
  paraglideMiddleware(event.request, ({ request: localizedRequest, locale }) => {
    event.request = localizedRequest;
    return resolve(event, {
      transformPageChunk: ({ html }) =>
        html
          .replace('%paraglide.lang%', locale)
          .replace('%paraglide.dir%', getTextDirection(locale)),
    });
  });

const auth: Handle = async ({ event, resolve }) => {
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

export const handle = sequence(i18n, auth);
