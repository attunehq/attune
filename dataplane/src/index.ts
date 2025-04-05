import path from 'node:path';

export default {
	async fetch(request, env, ctx): Promise<Response> {
		const url = new URL(request.url);

		// TODO: Do we need to handle some of these methods to be HTTP spec
		// compliant? Do any package manager clients require responses to things
		// like HEAD or OPTIONS or TRACE?
		if (request.method !== 'GET') {
			return new Response(null, { status: 405 });
		}

		// TODO: If we're going to do atomic release updates, this logic will need
		// to be a little smarter. Maybe store the current release ID in KV, and
		// check for the latest release ID given a hostname?
		const key = path.join(url.hostname, url.pathname);
		console.log({ key, msg: 'loading object' });
		const object = await env.R2.get(key);
		if (!object) {
			return new Response(null, { status: 404 });
		}

		const headers = new Headers();
		object.writeHttpMetadata(headers);
		headers.set('etag', object.httpEtag);

		return new Response(object.body, { headers });
	},
} satisfies ExportedHandler<Env>;
