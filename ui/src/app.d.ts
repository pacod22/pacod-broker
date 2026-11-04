import type { Broker } from '#lib/brokers.ts';
import type { ApiClient } from '#lib/server/api.ts';
import type { IdSession } from '@pacod/id-sveltekit';

export interface Session {
	mode: 'open' | 'oidc';
	sub?: string | null;
	name?: string | null;
	organisation_id?: string | null;
	permissions: string[];
	broker_id: string;
	version: string;
}

declare global {
	namespace App {
		interface Locals {
			/** The admin API of the broker chosen, with this request's bearer. */
			api: ApiClient;
			token: string | null;
			session?: IdSession | null;
			brokers: Broker[];
			broker: Broker;
			/** Who is signed in and what they may do, as the broker says. */
			principal: Session | null;
			/** Set when the broker could not be asked. */
			apiProblem: string | null;
		}
	}
}

export {};
