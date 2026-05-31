<script lang="ts">
  import { LoaderCircle } from 'lucide-svelte';
  import { appState } from '$lib/stores/app-state.svelte';

  let { children } = $props();
</script>

{#if appState.phase === 'ready'}
  {@render children()}
{:else}
  <div class="init-overlay">
    <div class="init-card">
      <div class="init-logo">
        <svg width="48" height="48" viewBox="0 0 48 48" fill="none">
          <rect width="48" height="48" rx="12" fill="#2563eb" />
          <path d="M14 16h6l4 16-4-16h6l4 16M32 16v16" stroke="white" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      </div>
      <h2 class="init-title">Ragtag</h2>

      {#if appState.phase === 'error'}
        <div class="init-error">
          <p>{appState.errorMessage}</p>
          <button class="retry-btn" onclick={() => window.location.reload()}>
            Retry
          </button>
        </div>
      {:else if appState.phase === 'downloading_model'}
        <div class="init-progress">
          <div class="progress-bar-track">
            <div 
              class="progress-bar-fill" 
              style="width: {appState.downloadProgress}%"
            ></div>
          </div>
          <p class="progress-text">{appState.downloadMessage || 'Downloading model...'}</p>
        </div>
      {:else}
        <div class="init-spinner">
          <span class="spinner-wrap">
            <LoaderCircle size={32} />
          </span>
          <p class="init-text">Initializing Ragtag...</p>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .init-overlay {
    position: fixed;
    inset: 0;
    background-color: #0f172a;
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 9999;
  }

  .init-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 1.5rem;
    text-align: center;
  }

  .init-logo {
    opacity: 0.9;
  }

  .init-title {
    font-size: 1.5rem;
    font-weight: 700;
    color: #f3f4f6;
    margin: 0;
    letter-spacing: -0.025em;
  }

  .init-spinner {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.75rem;
  }

  .spinner-wrap {
    animation: spin 1s linear infinite;
    color: #3b82f6;
    display: flex;
  }

  @keyframes spin {
    from { transform: rotate(0deg); }
    to { transform: rotate(360deg); }
  }

  .init-text {
    color: #9ca3af;
    font-size: 0.875rem;
    margin: 0;
  }

  .init-progress {
    width: 300px;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .progress-bar-track {
    width: 100%;
    height: 6px;
    background-color: #1e293b;
    border-radius: 3px;
    overflow: hidden;
  }

  .progress-bar-fill {
    height: 100%;
    background-color: #3b82f6;
    border-radius: 3px;
    transition: width 0.3s ease;
  }

  .progress-text {
    color: #9ca3af;
    font-size: 0.8125rem;
    margin: 0;
    word-break: break-word;
  }

  .init-error {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 1rem;
  }

  .init-error p {
    color: #f87171;
    font-size: 0.875rem;
    margin: 0;
    max-width: 320px;
  }

  .retry-btn {
    padding: 0.5rem 1.5rem;
    background-color: #3b82f6;
    color: white;
    border: none;
    border-radius: 0.375rem;
    font-size: 0.875rem;
    font-weight: 500;
    cursor: pointer;
    transition: background-color 0.2s;
  }

  .retry-btn:hover {
    background-color: #2563eb;
  }
</style>
