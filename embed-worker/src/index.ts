export default {
	async fetch(request, env): Promise<Response> {
		const url = new URL(request.url);

		if (url.pathname !== '/embed') {
			return new Response('Not Found', { status: 404 });
		}

		const clientIp = request.headers.get('CF-Connecting-IP') ?? 'unknown';
		const { success } = await env.EMBED_RATE_LIMITER.limit({ key: clientIp });
		if (!success) {
			return new Response('Too Many Requests', { status: 429 });
		}

		const rawVideoId = url.searchParams.get('v') ?? '';
		const safeVideoId = rawVideoId.replace(/[^a-zA-Z0-9_-]/g, '');

		if (!safeVideoId) {
			return new Response('Missing or invalid video id', { status: 400 });
		}

		const html = `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <meta name="referrer" content="strict-origin-when-cross-origin">
  <title>Sonic Player</title>
  <style>
    html, body {
      margin: 0;
      padding: 0;
      width: 100%;
      height: 100%;
      background-color: #000;
      overflow: hidden;
      display: flex;
      align-items: center;
      justify-content: center;
    }
    iframe {
      width: 100%;
      height: 100%;
      border: 0;
    }
  </style>
</head>
<body>
  <iframe
    src="https://www.youtube.com/embed/${safeVideoId}?autoplay=1&rel=0&playsinline=1"
    allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share"
    allowfullscreen
    referrerpolicy="strict-origin-when-cross-origin"
  ></iframe>
</body>
</html>`;

		return new Response(html, {
			headers: {
				'Content-Type': 'text/html; charset=utf-8',
				'Referrer-Policy': 'strict-origin-when-cross-origin',
				'Access-Control-Allow-Origin': '*',
				'Cache-Control': 'no-store',
			},
		});
	},
} satisfies ExportedHandler<Env>;
