import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
	plugins: [sveltekit()],
	clearScreen: false,
	server: {
		port: 5173,
		strictPort: true,
		watch: {
			// Tell vite to ignore watching `src-tauri`
			ignored: ['**/src-tauri/**']
		}
	}
});
