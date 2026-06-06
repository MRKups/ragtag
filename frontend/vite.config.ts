import { svelte } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vite';
import path from 'path';

export default defineConfig({
	plugins: [svelte()],
	publicDir: 'static',
	clearScreen: false,
	resolve: {
		alias: {
			$lib: path.resolve('./src/lib')
		}
	},
	server: {
		port: 5173,
		strictPort: true,
		watch: {
			ignored: ['**/src-tauri/**']
		}
	}
});
