<script lang="ts">
	import { onMount } from 'svelte';
	import { invoke } from '@tauri-apps/api/core';

	let pingResponse = $state('Pinging backend...');

	onMount(async () => {
		try {
			pingResponse = await invoke<string>('ping');
		} catch (error) {
			pingResponse = `Error: ${error}`;
		}
	});
</script>

<main>
	<h1>Ragtag Architecture Prototype</h1>
	<div class="card">
		<h2>Tauri IPC Status:</h2>
		<p class="status" class:success={pingResponse === 'pong'}>
			{pingResponse}
		</p>
	</div>
</main>

<style>
	main {
		font-family: system-ui, -apple-system, sans-serif;
		padding: 2rem;
		max-width: 800px;
		margin: 0 auto;
	}
	.card {
		padding: 1.5rem;
		border-radius: 8px;
		background: #f4f4f5;
		margin-top: 2rem;
	}
	.status {
		font-family: monospace;
		font-size: 1.2rem;
		font-weight: bold;
		color: #ef4444; /* red for error */
	}
	.status.success {
		color: #10b981; /* green for success */
	}
</style>
