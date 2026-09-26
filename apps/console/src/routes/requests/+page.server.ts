import type { PageServerLoad, Actions } from './$types';
import { listRequests, getRequest } from '$lib/server/vkdg/client';
import type { RequestSummary } from '$lib/server/vkdg/client';

export const load: PageServerLoad = async ({ request }) => {
  const cookie = request.headers.get('cookie') ?? '';
  const requests = await listRequests(cookie, 50);
  return { requests };
};

export const actions: Actions = {
  search: async ({ request }) => {
    const cookie = request.headers.get('cookie') ?? '';
    const form = await request.formData();
    const id = form.get('id')?.toString() ?? '';
    if (!id.trim()) return { error: 'Request ID is required' };
    try {
      const detail: RequestSummary = await getRequest(cookie, id);
      return { detail };
    } catch (e) {
      return { error: (e as Error).message };
    }
  },
};
