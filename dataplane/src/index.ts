/**
 * Welcome to Cloudflare Workers! This is your first worker.
 *
 * - Run `npm run dev` in your terminal to start a development server
 * - Open a browser tab at http://localhost:8787/ to see your worker in action
 * - Run `npm run deploy` to publish your worker
 *
 * Bind resources to your worker in `wrangler.jsonc`. After adding bindings, a type definition for the
 * `Env` object can be regenerated with `npm run cf-typegen`.
 *
 * Learn more at https://developers.cloudflare.com/workers/
 */

import path from 'path';

import { Pool } from 'pg';
import { Kysely, PostgresDialect } from 'kysely';

import { DB } from './db';

export default {
	async fetch(request, env, ctx): Promise<Response> {
		if (request.method !== 'GET') {
			return new Response('Method not allowed', { status: 405 });
		}

		const db = new Kysely<DB>({
			dialect: new PostgresDialect({
				pool: new Pool({ connectionString: env.ATTUNE_RDS_HYPERDRIVE.connectionString }),
			}),
		});

		const url = new URL(request.url);
		const repo = await db.selectFrom('debian_repository').where('uri', '=', url.hostname).select('s3_prefix').executeTakeFirst();
		if (!repo) {
			return new Response('Not found', { status: 404 });
		}

		const key = path.join(repo.s3_prefix, url.pathname);
		console.log({ message: 'loading object', key });
		const object = await env.ATTUNE_R2_BUCKET.get(key);
		if (!object) {
			return new Response('Not found', { status: 404 });
		}

		return new Response(object.body, {
			headers: {
				'Content-Type': object.httpMetadata?.contentType ?? 'application/octet-stream',
			},
		});
	},
} satisfies ExportedHandler<Env>;
