import type { PageServerLoad, Actions } from './$types';
import { listRoutes, previewRoute } from '$lib/server/vkdg/client';
import type { RoutePreview } from '$lib/server/vkdg/client';

export const load: PageServerLoad = async ({ request }) => {
  const cookie = request.headers.get('cookie') ?? '';
  const routes = await listRoutes(cookie);
  return { routes };
};

export const actions: Actions = {
  preview: async ({ request }) => {
    const cookie = request.headers.get('cookie') ?? '';
    const form = await request.formData();
    const model = form.get('model')?.toString() ?? '';
    if (!model.trim()) return { error: 'Model is required' };
    try {
      const preview: RoutePreview = await previewRoute(cookie, model);
      return { preview };
    } catch (e) {
      return { error: (e as Error).message };
    }
  },
};
