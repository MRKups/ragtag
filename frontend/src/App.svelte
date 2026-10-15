<script lang="ts">
  import { onMount } from 'svelte';
  import { MessageSquare, Files, Settings as SettingsIcon, Sun, Moon } from '@lucide/svelte';
  import rtLogoUrl from '$lib/assets/rt-logo.svg';
  import { api } from '$lib/api';
  import { settingsState } from '$lib/stores/settings.svelte';
  import { appState } from '$lib/stores/app-state.svelte';
  import AppInitializer from '$lib/components/AppInitializer.svelte';
  import ErrorDialog from '$lib/components/ErrorDialog.svelte';
  import { errorDialog } from '$lib/stores/error-dialog.svelte';
  import { router } from '$lib/router.svelte';
  import QueryPage from './pages/QueryPage.svelte';
  import DocumentsPage from './pages/DocumentsPage.svelte';
  import SettingsPage from './pages/SettingsPage.svelte';

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

  function preventDroppedFileNavigation(event: DragEvent) {
    if (event.dataTransfer?.types.includes('Files')) {
      event.preventDefault();
    }
  }

  onMount(() => {
    void api.getPlatform().then((platform) => {
      document.documentElement.classList.add(`platform-${platform}`);
    }).catch(() => {
      document.documentElement.classList.add('platform-macos');
    });

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
    window.addEventListener('dragover', preventDroppedFileNavigation);
    window.addEventListener('drop', preventDroppedFileNavigation);


    void settingsState.load().then(() => appState.init());

    return () => {
      window.removeEventListener('error', handleError);
      window.removeEventListener('unhandledrejection', handleUnhandledRejection);
      window.removeEventListener('dragover', preventDroppedFileNavigation);
      window.removeEventListener('drop', preventDroppedFileNavigation);
    };
  });
</script>

<AppInitializer>
  <div class="app-container">
    <header class="header">
      <div class="drag-surface" data-tauri-drag-region aria-hidden="true"></div>
      <div class="brand">
        <button class="logo-link" onclick={() => router.navigate('query')}>
          <img src={rtLogoUrl} alt="Ragtag" class="logo-img" width="36" height="36" />
          <h1>Ragtag</h1>
        </button>
      </div>

      <nav class="nav-menu">
        <button
          class="nav-btn"
          class:active={router.current === 'query'}
          onclick={() => router.navigate('query')}
        >
          <MessageSquare size={18} />
          <span>Query</span>
        </button>
        <button
          class="nav-btn"
          class:active={router.current === 'documents'}
          onclick={() => router.navigate('documents')}
        >
          <Files size={18} />
          <span>Documents</span>
        </button>
        <button
          class="nav-btn"
          class:active={router.current === 'settings'}
          onclick={() => router.navigate('settings')}
        >
          <SettingsIcon size={18} />
          <span>Settings</span>
        </button>
      </nav>

      <div class="header-actions">
        <button
          type="button"
          class="theme-toggle-btn"
          onclick={() => void settingsState.toggleTheme()}
          title={`Switch to ${settingsState.theme === 'dark' ? 'light' : 'dark'} mode`}
          aria-label="Toggle theme"
        >
          {#if settingsState.theme === 'dark'}
            <Sun size={18} />
          {:else}
            <Moon size={18} />
          {/if}
        </button>
      </div>
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
  :root {
    --bg-app: #121214;
    --bg-header: #18181b;
    --bg-panel: #121214;
    --bg-panel-alt: #18181b;
    --bg-card: #1e1e22;
    --bg-card-subtle: #161619;
    --bg-card-hover: #26262b;
    --bg-input: #16161a;
    --border-color: #27272a;
    --border-card: #2e2e33;
    --border-card-hover: #3f3f46;
    --text-primary: #f4f4f5;
    --text-secondary: #a1a1aa;
    --text-muted: #71717a;
    --text-heading: #ffffff;
    --primary: #3b82f6;
    --primary-hover: #2563eb;
    --primary-ring: #3b82f6;
    --primary-soft: rgba(59, 130, 246, 0.12);
    --primary-border: rgba(59, 130, 246, 0.3);
    --citation-badge-bg: rgba(59, 130, 246, 0.15);
    --citation-badge-text: #60a5fa;
    --citation-card-bg: #18181b;
    --citation-card-hover: #222226;
    --scrollbar-thumb: rgba(161, 161, 170, 0.28);
    --scrollbar-thumb-hover: rgba(161, 161, 170, 0.5);
    --shadow-card: 0 4px 6px -1px rgba(0, 0, 0, 0.35);
  }

  :root[data-theme="light"] {
    --bg-app: #f8fafc;
    --bg-header: #ffffff;
    --bg-panel: #f8fafc;
    --bg-panel-alt: #f1f5f9;
    --bg-card: #ffffff;
    --bg-card-subtle: #f8fafc;
    --bg-card-hover: #f1f5f9;
    --bg-input: #ffffff;
    --border-color: #e2e8f0;
    --border-card: #e2e8f0;
    --border-card-hover: #cbd5e1;
    --text-primary: #0f172a;
    --text-secondary: #475569;
    --text-muted: #64748b;
    --text-heading: #0f172a;
    --primary: #2563eb;
    --primary-hover: #1d4ed8;
    --primary-ring: #2563eb;
    --primary-soft: rgba(37, 99, 235, 0.1);
    --primary-border: rgba(59, 130, 246, 0.25);
    --citation-badge-bg: #dbeafe;
    --citation-badge-text: #1d4ed8;
    --citation-card-bg: #ffffff;
    --citation-card-hover: #f8fafc;
    --scrollbar-thumb: rgba(100, 116, 139, 0.35);
    --scrollbar-thumb-hover: rgba(100, 116, 139, 0.6);
    --shadow-card: 0 4px 6px -1px rgba(0, 0, 0, 0.06);
  }

  :global(*) {
    box-sizing: border-box;
    scrollbar-width: thin;
    scrollbar-color: var(--scrollbar-thumb) transparent;
  }

  :global(*::-webkit-scrollbar) {
    width: 6px;
    height: 6px;
  }

  :global(*::-webkit-scrollbar-track) {
    background: transparent;
  }

  :global(*::-webkit-scrollbar-thumb) {
    background: var(--scrollbar-thumb);
    border-radius: 9999px;
  }

  :global(*::-webkit-scrollbar-thumb:hover) {
    background: var(--scrollbar-thumb-hover);
  }

  :global(body) {
    margin: 0;
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, 'Open Sans', 'Helvetica Neue', sans-serif;
    background-color: var(--bg-app);
    color: var(--text-primary);
    overflow: hidden;
  }

  .app-container {
    display: flex;
    flex-direction: column;
    height: 100dvh;
    width: 100%;
    overflow: hidden;
    background-color: var(--bg-app);
  }

  .header {
    position: relative;
    height: 48px;
    border-bottom: 1px solid var(--border-color);
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 2rem;
    background-color: var(--bg-header);
    flex-shrink: 0;
    width: 100%;
    isolation: isolate;
    z-index: 10;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    width: 300px;
    padding-left: 0.5rem;
  }

  :global(.platform-macos) .brand {
    padding-left: 4.5rem;
  }

  .logo-link {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    text-decoration: none;
    background: none;
    border: none;
    cursor: pointer;
    padding: 0;
    color: inherit;
    font: inherit;
  }

  .header-actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    width: 300px;
    padding-right: 0.5rem;
  }

  .theme-toggle-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border-radius: 0.5rem;
    border: 1px solid var(--border-color);
    background: var(--bg-card);
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .theme-toggle-btn:hover {
    color: var(--text-primary);
    border-color: var(--border-card-hover);
    background: var(--bg-card-hover);
  }

  .drag-surface {
    position: absolute;
    inset: 0;
    z-index: 0;
  }

  .brand,
  .nav-menu,
  .header-actions {
    position: relative;
    z-index: 1;
    pointer-events: none;
  }

  .logo-link,
  .nav-btn,
  .theme-toggle-btn {
    pointer-events: auto;
  }

  .logo-img {
    width: 36px;
    height: 36px;
    display: block;
    flex-shrink: 0;
  }

  h1 {
    font-size: 1.125rem;
    font-weight: 700;
    color: var(--text-heading);
    margin: 0;
    letter-spacing: -0.025em;
    white-space: nowrap;
  }

  .nav-menu {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    background: transparent;
    padding: 0;
    border: none;
  }

  .nav-btn {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.45rem 0.9rem;
    border-radius: 0.5rem;
    border: 1px solid transparent;
    background: transparent;
    color: var(--text-secondary);
    font-size: 0.875rem;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .nav-btn:hover {
    color: var(--text-primary);
    background-color: var(--bg-card-hover);
  }

  .nav-btn.active {
    background-color: var(--primary);
    color: #ffffff;
    box-shadow: 0 1px 2px 0 rgba(0, 0, 0, 0.05);
  }

  .nav-btn.active:hover {
    background-color: var(--primary-hover);
    color: #ffffff;
  }

  .main-content {
    display: flex;
    flex: 1;
    overflow: hidden;
    width: 100%;
  }

</style>
