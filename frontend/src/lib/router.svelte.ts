export type Route = 'query' | 'documents' | 'settings';

class HashRouter {
	current = $state<Route>('query');

	constructor() {
		this.#sync();
		if (typeof window !== 'undefined') {
			window.addEventListener('hashchange', () => this.#sync());
		}
	}

	navigate(route: Route) {
		const hash = route === 'query' ? '/' : `/${route}`;
		window.location.hash = hash;
	}

	#sync() {
		const hash = window.location.hash.slice(1) || '/';
		this.current = hash === '/documents' ? 'documents' :
		               hash === '/settings' ? 'settings' : 'query';
	}
}

export const router = new HashRouter();
