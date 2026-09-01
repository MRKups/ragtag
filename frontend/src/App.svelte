<script lang="ts">
  import { onMount } from 'svelte';
  import { Wrench, MessageSquare, Files, Settings as SettingsIcon } from '@lucide/svelte';
  import { settingsState } from '$lib/stores/settings.svelte';
  import { appState } from '$lib/stores/app-state.svelte';
  import AppInitializer from '$lib/components/AppInitializer.svelte';
  import ErrorDialog from '$lib/components/ErrorDialog.svelte';
  import { errorDialog } from '$lib/stores/error-dialog.svelte';
  import { router, type Route } from '$lib/router.svelte';
  import QueryPage from './pages/QueryPage.svelte';
  import DocumentsPage from './pages/DocumentsPage.svelte';
  import SettingsPage from './pages/SettingsPage.svelte';


  function navTo(route: Route) {
    if (typeof document.startViewTransition === 'function') {
      document.startViewTransition(() => router.navigate(route));
    } else {
      router.navigate(route);
    }
  }

  function reportUnexpectedClientError() {
    errorDialog.report({
      dedupeKey: 'unexpected-client-error',
      title: 'Something went wrong',
      message: 'An unexpected error occurred in the app.',
      guidance: 'Try again. If the problem continues, restart Ragtag.',
    });
  }

  function isAbortError(reason: unknown) {
    return typeof reason === 'object'
      && reason !== null
      && 'name' in reason
      && reason.name === 'AbortError';
  }

  onMount(() => {
    const handleError = (event: ErrorEvent) => {
      event.preventDefault();
      reportUnexpectedClientError();
    };
    const handleUnhandledRejection = (event: PromiseRejectionEvent) => {
      if (isAbortError(event.reason)) return;

      event.preventDefault();
      reportUnexpectedClientError();
    };

    window.addEventListener('error', handleError);
    window.addEventListener('unhandledrejection', handleUnhandledRejection);

    void settingsState.load().then(() => appState.init());

    return () => {
      window.removeEventListener('error', handleError);
      window.removeEventListener('unhandledrejection', handleUnhandledRejection);
    };
  });
</script>

<AppInitializer>
  <div class="app-container">
    <header class="header">
      <div class="drag-surface" data-tauri-drag-region aria-hidden="true"></div>
      <div class="brand">
        <button class="logo-link" onclick={() => navTo('query')}>
          <div class="logo">
            <Wrench color="white" size={20} />
          </div>
          <h1>Ragtag V1</h1>
        </button>
      </div>

      <nav class="nav-menu">
        <button
          class="nav-btn"
          class:active={router.current === 'query'}
          onclick={() => navTo('query')}
        >
          <MessageSquare size={18} />
          <span>Query</span>
        </button>
        <button
          class="nav-btn"
          class:active={router.current === 'documents'}
          onclick={() => navTo('documents')}
        >
          <Files size={18} />
          <span>Documents</span>
        </button>
        <button
          class="nav-btn"
          class:active={router.current === 'settings'}
          onclick={() => navTo('settings')}
        >
          <SettingsIcon size={18} />
          <span>Settings</span>
        </button>
      </nav>

      <div class="header-spacer"></div>
    </header>

    <main class="main-content">
      {#key router.current}
        {#if router.current === 'query'}
          <QueryPage />
        {:else if router.current === 'documents'}
          <DocumentsPage />
        {:else if router.current === 'settings'}
          <SettingsPage />
        {/if}
      {/key}
    </main>
  </div>
  <ErrorDialog />
</AppInitializer>

<style>
  :global(*) {
    box-sizing: border-box;
  }

  :global(body) {
    margin: 0;
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, 'Open Sans', 'Helvetica Neue', sans-serif;
    background-color: #111827;
    color: #f3f4f6;
    overflow: hidden;
  }

  .app-container {
    display: flex;
    flex-direction: column;
    height: 100dvh;
    width: 100%;
    overflow: hidden;
  }

  .header {
    position: relative;
    height: 48px;
    border-bottom: 1px solid #1e293b;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 2rem;
    background-color: #0f172a;
    flex-shrink: 0;
    width: 100%;
    isolation: isolate;
    z-index: 10;
    view-transition-name: app-header;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    width: 300px;
    padding-left: 4.5rem;
  }

  .logo-link {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    text-decoration: none;
    background: none;
    border: none;
    cursor: pointer;
    padding: 0;
    color: inherit;
    font: inherit;
  }

  .header-spacer {
    width: 300px;
  }

  .drag-surface {
    position: absolute;
    inset: 0;
    z-index: 0;
  }

  .brand,
  .nav-menu,
  .header-spacer {
    position: relative;
    z-index: 1;
  }

  .brand,
  .nav-menu,
  .header-spacer {
    pointer-events: none;
  }

  .logo-link,
  .nav-btn {
    pointer-events: auto;
  }

  .logo {
    padding: 0.5rem;
    background-color: #2563eb;
    border-radius: 0.5rem;
    box-shadow: 0 4px 6px -1px rgba(0, 0, 0, 0.1);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  h1 {
    font-size: 1.125rem;
    font-weight: 700;
    color: white;
    margin: 0;
    letter-spacing: -0.025em;
    white-space: nowrap;
  }

  .nav-menu {
    display: flex;
    gap: 0.5rem;
    background-color: #1e293b;
    padding: 0.25rem;
    border-radius: 0.5rem;
    border: 1px solid #374151;
  }

  .nav-btn {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.5rem 1rem;
    border-radius: 0.375rem;
    border: none;
    background: transparent;
    color: #9ca3af;
    font-size: 0.875rem;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.2s;
  }

  .nav-btn:hover {
    color: white;
  }

  .nav-btn.active {
    background-color: #3b82f6;
    color: white;
    box-shadow: 0 1px 2px 0 rgba(0, 0, 0, 0.05);
  }

  .main-content {
    display: flex;
    flex: 1;
    overflow: hidden;
    width: 100%;
    view-transition-name: main-content;
  }

  ::view-transition-old(main-content) {
    animation: fade-out 150ms ease-out forwards;
  }

  ::view-transition-new(main-content) {
    animation: fade-in 150ms ease-out forwards;
  }

  @keyframes fade-out {
    from { opacity: 1; }
    to { opacity: 0; }
  }

  @keyframes fade-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }
</style>
