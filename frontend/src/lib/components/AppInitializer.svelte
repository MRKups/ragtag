<script lang="ts">
  import { LoaderCircle } from '@lucide/svelte';
  import { appState } from '$lib/stores/app-state.svelte';
  import rtLogoUrl from '$lib/assets/rt-logo.svg';

  let { children } = $props();
</script>

{#if appState.phase === 'ready'}
  {@render children()}
{:else}
  <div class="init-overlay">
    <div class="init-drag-surface" data-tauri-drag-region aria-hidden="true"></div>
    <div class="init-card">
      <div class="init-logo" data-tauri-drag-region>
        <img src={rtLogoUrl} alt="Ragtag" width="64" height="64" class="init-logo-img" />
      </div>
      <h2 class="init-title" data-tauri-drag-region>Ragtag</h2>

      {#if appState.phase === 'error'}
        <div class="init-error">
          <p>{appState.errorMessage}</p>
          <button class="retry-btn" onclick={() => window.location.reload()}>
            Retry
          </button>
        </div>
      {:else if appState.phase === 'downloading_model'}
        <div class="init-progress">
          <div
            class="progress-bar-track"
            role="progressbar"
            aria-label="Model initialization progress"
            aria-valuemin="0"
            aria-valuemax="100"
            aria-valuenow={appState.downloadIsIndeterminate ? undefined : appState.downloadProgress}
          >
            {#if appState.downloadIsIndeterminate}
              <div class="progress-bar-fill indeterminate"></div>
            {:else}
              <div class="progress-bar-fill" style="width: {appState.downloadProgress}%"></div>
            {/if}
          </div>
          <p class="progress-text">{appState.downloadMessage || 'Downloading model...'}</p>
          <button
            type="button"
            class="cancel-download-btn"
            onclick={() => void appState.cancelModelDownload()}
          >
            Cancel Download
          </button>
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
    background-color: var(--bg-app, #121214);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 9999;
  }

  .init-drag-surface {
    position: absolute;
    inset: 0;
    z-index: 1;
  }

  .init-card {
    position: relative;
    z-index: 2;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 1.5rem;
    text-align: center;
    pointer-events: none;
  }

  .init-card > * {
    pointer-events: auto;
  }

  .init-logo {
    opacity: 0.95;
  }

  .init-logo-img {
    width: 72px;
    height: 72px;
    display: block;
  }

  .init-title {
    font-size: 1.5rem;
    font-weight: 700;
    color: var(--text-primary, #f4f4f5);
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
    color: var(--primary, #3b82f6);
    display: flex;
  }

  @keyframes spin {
    from { transform: rotate(0deg); }
    to { transform: rotate(360deg); }
  }

  .init-text {
    color: var(--text-secondary, #a1a1aa);
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
    background-color: var(--border-color, #27272a);
    border-radius: 3px;
    overflow: hidden;
  }

  .progress-bar-fill {
    height: 100%;
    background-color: var(--primary, #3b82f6);
    border-radius: 3px;
    transition: width 0.3s ease;
  }

  .progress-bar-fill.indeterminate {
    width: 40%;
    animation: indeterminate-progress 1.2s ease-in-out infinite;
  }

  @keyframes indeterminate-progress {
    from { transform: translateX(-100%); }
    to { transform: translateX(350%); }
  }

  .progress-text {
    color: var(--text-secondary, #a1a1aa);
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
    background-color: var(--primary, #3b82f6);
    color: white;
    border: none;
    border-radius: 0.375rem;
    font-size: 0.875rem;
    font-weight: 500;
    cursor: pointer;
    transition: background-color 0.2s;
  }

  .retry-btn:hover {
    background-color: var(--primary-hover, #2563eb);
  }

  .cancel-download-btn {
    align-self: center;
    margin-top: 0.5rem;
    padding: 0.375rem 1rem;
    background-color: transparent;
    color: var(--text-secondary, #a1a1aa);
    border: 1px solid var(--border-color, #27272a);
    border-radius: 0.375rem;
    font-size: 0.75rem;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.2s;
  }

  .cancel-download-btn:hover {
    color: var(--text-primary, #f4f4f5);
    border-color: var(--border-card-hover, #3f3f46);
    background-color: var(--bg-card-hover, #26262b);
  }
</style>
