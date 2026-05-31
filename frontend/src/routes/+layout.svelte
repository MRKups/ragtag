<script lang="ts">
  import { onMount } from 'svelte';
  import { Wrench, MessageSquare, Files, Settings as SettingsIcon } from 'lucide-svelte';
  import { page } from '$app/stores';
  import { settingsState } from '$lib/stores/settings.svelte';
  import { appState } from '$lib/stores/app-state.svelte';
  import AppInitializer from '$lib/components/AppInitializer.svelte';

  let { children } = $props();

  onMount(async () => {
    await settingsState.load();
    appState.init();
  });
</script>

<AppInitializer>
  <div class="app-container">
    <header class="header">
      <div class="brand">
        <a href="/" class="logo-link">
          <div class="logo">
            <Wrench color="white" size={20} />
          </div>
          <h1>Ragtag V1</h1>
        </a>
      </div>
      
      <nav class="nav-menu">
        <a 
          href="/" 
          class="nav-btn {String($page.url.pathname) === '/' ? 'active' : ''}"
        >
          <MessageSquare size={18} />
          <span>Query</span>
        </a>
        <a 
          href="/documents" 
          class="nav-btn {String($page.url.pathname) === '/documents' ? 'active' : ''}"
        >
          <Files size={18} />
          <span>Documents</span>
        </a>
        <a 
          href="/settings" 
          class="nav-btn {String($page.url.pathname) === '/settings' ? 'active' : ''}"
        >
          <SettingsIcon size={18} />
          <span>Settings</span>
        </a>
      </nav>

      <div class="header-spacer"></div>
    </header>

    <main class="main-content">
      {@render children()}
    </main>
  </div>
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
    height: 100vh; /* Fallback */
    height: 100dvh;
    width: 100%;
    overflow: hidden;
  }

  /* Header */
  .header {
    height: 64px;
    border-bottom: 1px solid #1e293b;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 2rem;
    background-color: #0f172a;
    flex-shrink: 0;
    width: 100%;
    z-index: 10;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    width: 300px;
  }
  
  .logo-link {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    text-decoration: none;
  }

  .header-spacer {
    width: 300px; /* Matches brand width to ensure nav is perfectly centered */
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

  /* Navigation */
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
    text-decoration: none;
  }

  .nav-btn:hover {
    color: white;
  }

  .nav-btn.active {
    background-color: #3b82f6;
    color: white;
    box-shadow: 0 1px 2px 0 rgba(0, 0, 0, 0.05);
  }

  /* Main Content */
  .main-content {
    display: flex;
    flex: 1;
    overflow: hidden;
    width: 100%;
  }
</style>
