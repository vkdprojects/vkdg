import type { RequestHandler } from './$types';

const DATA_URL = process.env.VKDG_DATA_URL ?? 'http://127.0.0.1:8080';

export const POST: RequestHandler = async ({ request, locals }) => {
  if (!locals.user) return new Response('Unauthorized', { status: 401 });

  const body = await request.json();

  const upstream = await fetch(`${DATA_URL}/v1/chat/completions`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'Authorization': `Bearer ${process.env.VKDG_PLAYGROUND_KEY ?? 'dev'}`,
      'X-VKDG-Connection': body.connection_id ?? '',
    },
    body: JSON.stringify({
      model: body.model,
      messages: body.messages,
      temperature: body.temperature,
      stream: body.stream,
    }),
  });

  return new Response(upstream.body, {
    status: upstream.status,
    headers: {
      'Content-Type': upstream.headers.get('Content-Type') ?? 'application/json',
      'Cache-Control': 'no-cache',
    },
  });
};
