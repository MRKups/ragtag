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
		const next: Route = hash === '/documents' ? 'documents' :
		                    hash === '/settings' ? 'settings' : 'query';
		if (this.current === next) return;

		if (typeof document !== 'undefined' && 'startViewTransition' in document) {
			document.startViewTransition(() => {
				this.current = next;
			});
		} else {
			this.current = next;
		}
	}
}

export const router = new HashRouter();
