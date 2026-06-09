<script lang="ts">
  import { onMount } from 'svelte';
  import { settingsState } from '$lib/stores/settings.svelte';
  import { api } from '$lib/api';
  import { Cpu, Database, Link, Key, BrainCircuit } from 'lucide-svelte';
  import Toggle from './Toggle.svelte';

const modes = [
  { value: 'local', label: 'Local (In-process)' },
  { value: 'external', label: 'External (OpenAI-compatible)' }
];

let apiKeyValue = $state('');
  let apiKeyExists = $state(false);
  let apiKeySaved = $state(false);

  // Storage management
  let storageInfo = $state<{
    database_bytes: number;
    vector_store_bytes: number;
    documents_bytes: number;
    models_bytes: number;
  } | null>(null);
  let storageMessage = $state('');
  let showResetConfirm = $state(false);
  let resetDeleteModels = $state(false);

  onMount(async () => {
    try {
      apiKeyExists = await api.hasApiKey();
    } catch (e) {
      console.error('Failed to check API key:', e);
    }
    await loadStorageInfo();
  });

  function updateSetting(key: string, value: string) {
    settingsState.update(key, value);
  }

  async function saveApiKey() {
    if (!apiKeyValue.trim()) return;
    try {
      await api.setApiKey(apiKeyValue.trim());
      apiKeyExists = true;
      apiKeySaved = true;
      apiKeyValue = '';
      setTimeout(() => (apiKeySaved = false), 2000);
    } catch (e) {
      console.error('Failed to save API key:', e);
    }
  }

  async function clearApiKey() {
    try {
      await api.deleteApiKey();
      apiKeyExists = false;
      apiKeyValue = '';
    } catch (e) {
      console.error('Failed to delete API key:', e);
    }
  }

  function formatBytes(bytes: number): string {
    if (bytes === 0) return '0 B';
    const units = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(1024));
    return `${(bytes / Math.pow(1024, i)).toFixed(1)} ${units[i]}`;
  }

  async function loadStorageInfo() {
    try {
      storageInfo = await api.getStorageInfo();
    } catch (e) {
      console.error('Failed to load storage info:', e);
    }
  }

  async function doCompact() {
    try {
      await api.compactVectorStore();
      storageMessage = 'Vector store compacted.';
      await loadStorageInfo();
    } catch (e) {
      storageMessage = `Compact failed: ${e}`;
    }
    setTimeout(() => (storageMessage = ''), 3000);
  }

  async function doVacuum() {
    try {
      await api.vacuumDatabase();
      storageMessage = 'Database vacuumed.';
      await loadStorageInfo();
    } catch (e) {
      storageMessage = `VACUUM failed: ${e}`;
    }
    setTimeout(() => (storageMessage = ''), 3000);
  }

  async function doReset() {
    try {
      await api.resetAllData(resetDeleteModels);
      storageMessage = 'Reset pending. Restarting...';
      setTimeout(() => window.location.reload(), 1500);
    } catch (e) {
      storageMessage = `Reset failed: ${e}`;
    }
  }
</script>

<div class="settings-container">
  <div class="header">
    <h2>Settings</h2>
    <p>Configure your application preferences</p>
  </div>

  <div class="settings-content">
    {#if settingsState.isLoading}
      <div class="loading">Loading settings...</div>
    {:else}
      {#if settingsState.loadError}
        <div class="error-message">{settingsState.loadError}</div>
      {/if}
      <div class="setting-group">
        <h3><Cpu size={18} /> Inference Configuration</h3>

        <div class="setting-field">
          <label for="inference_mode">Inference Mode</label>
          <select
            id="inference_mode"
            bind:value={settingsState.inference_mode}
            onchange={() => updateSetting('inference_mode', settingsState.inference_mode)}
          >
            {#each modes as mode}
              <option value={mode.value}>{mode.label}</option>
            {/each}
          </select>
          <p class="help-text">Where the language model runs. Local uses your computer's CPU/GPU.</p>
        </div>

        {#if settingsState.inference_mode === 'external'}
          <div class="setting-field">
            <label for="external_base_url">
              <Link size={14} class="mr-1 inline" />
              External Base URL
            </label>
            <input
              id="external_base_url"
              type="text"
              placeholder="http://localhost:11434/v1"
              bind:value={settingsState.external_base_url}
              onblur={() => { const v = settingsState.external_base_url.trim(); if (v) updateSetting('external_base_url', v); }}
            />
          </div>

          <div class="setting-field">
            <label for="external_api_key">
              <Key size={14} class="mr-1 inline" />
              API Key
            </label>
            {#if apiKeyExists}
              <div class="keyring-row">
                <span class="keyring-status">Stored in OS keychain</span>
                <button class="btn btn-small btn-danger" onclick={clearApiKey}>Clear Key</button>
              </div>
            {:else}
              <div class="keyring-row">
                <input
                  id="external_api_key"
                  type="password"
                  placeholder="sk-..."
                  bind:value={apiKeyValue}
                  onkeydown={(e) => e.key === 'Enter' && saveApiKey()}
                />
                <button class="btn btn-small" onclick={saveApiKey}>Save Key</button>
              </div>
            {/if}
            {#if apiKeySaved}
              <p class="success-text">API key saved securely.</p>
            {/if}
            <p class="help-text">Stored in the OS keychain, never in the database or on disk.</p>
          </div>
        {:else}
          <div class="setting-field">
            <label for="generation_model">
              <BrainCircuit size={14} class="mr-1 inline" />
              Hugging Face Model Repo
            </label>
            <input
              id="generation_model"
              type="text"
              placeholder="Qwen/Qwen3-1.7B-Instruct"
              bind:value={settingsState.generation_model}
              onblur={() => { const v = settingsState.generation_model.trim(); if (v) updateSetting('generation_model', v); }}
            />
            <p class="help-text">GGUF models are pre-quantized — ISQ does not apply in local mode.</p>
          </div>
          <div class="setting-field">
            <label for="gguf_filename">GGUF Filename</label>
            <input
              id="gguf_filename"
              type="text"
              placeholder="Qwen3-1.7B-Q4_K_M.gguf"
              bind:value={settingsState.gguf_filename}
              onblur={() => { const v = settingsState.gguf_filename.trim(); if (v) updateSetting('gguf_filename', v); }}
            />
            <p class="help-text">Filename of the GGUF file within the Hugging Face repo above.</p>
          </div>
        {/if}
      </div>

      <div class="setting-group">
        <h3><Database size={18} /> RAG Pipeline Configuration</h3>

        <Toggle
          bind:checked={settingsState.use_query_enhancement}
          label="Query Enhancement"
          description="Automatically expand queries with technical synonyms to improve search recall."
          onchange={() => updateSetting('use_query_enhancement', String(settingsState.use_query_enhancement))}
        />

        <div class="split-fields">
          <div class="setting-field">
            <label for="chunk_size">Chunk Size (Tokens)</label>
            <input
              id="chunk_size"
              type="number"
              bind:value={settingsState.chunk_size}
              onblur={() => updateSetting('chunk_size', settingsState.chunk_size.toString())}
            />
          </div>
          <div class="setting-field">
            <label for="chunk_overlap">Chunk Overlap</label>
            <input
              id="chunk_overlap"
              type="number"
              bind:value={settingsState.chunk_overlap}
              onblur={() => updateSetting('chunk_overlap', settingsState.chunk_overlap.toString())}
            />
          </div>
        </div>
        <p class="help-text mt-n2">Changes only apply to documents uploaded after the change.</p>
      </div>

      <div class="setting-group">
        <h3><Database size={18} /> Storage Management</h3>

        {#if storageInfo}
          <div class="storage-metrics">
            <div class="metric">
              <span class="metric-label">Database</span>
              <span class="metric-value">{formatBytes(storageInfo.database_bytes)}</span>
            </div>
            <div class="metric">
              <span class="metric-label">Vector Store</span>
              <span class="metric-value">{formatBytes(storageInfo.vector_store_bytes)}</span>
            </div>
            <div class="metric">
              <span class="metric-label">Documents</span>
              <span class="metric-value">{formatBytes(storageInfo.documents_bytes)}</span>
            </div>
            <div class="metric">
              <span class="metric-label">Models</span>
              <span class="metric-value">{formatBytes(storageInfo.models_bytes)}</span>
            </div>
          </div>
        {/if}

        <div class="storage-actions">
          <button class="btn" onclick={loadStorageInfo}>
            <Database size={14} /> Refresh
          </button>
          <button class="btn" onclick={doCompact}>Compact Vector Store</button>
          <button class="btn" onclick={doVacuum}>VACUUM Database</button>
        </div>

        <div class="storage-actions">
          <button class="btn btn-danger" onclick={() => (showResetConfirm = true)}>
            Reset All Data
          </button>
        </div>

        {#if showResetConfirm}
          <div class="reset-modal-overlay" role="button" tabindex="0" onclick={() => (showResetConfirm = false)} onkeydown={(e) => e.key === 'Escape' && (showResetConfirm = false)}>
            <div class="reset-modal" role="dialog" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
              <h4>Reset All Data?</h4>
              <p>This will delete all documents, vectors, and the database. The app must restart to apply.</p>
              <label class="checkbox-label">
                <input type="checkbox" bind:checked={resetDeleteModels} />
                Also delete downloaded models
              </label>
              <div class="reset-modal-actions">
                <button class="btn" onclick={() => (showResetConfirm = false)}>Cancel</button>
                <button class="btn btn-danger" onclick={doReset}>Reset & Restart</button>
              </div>
            </div>
          </div>
        {/if}

        {#if storageMessage}
          <p class="success-text">{storageMessage}</p>
        {/if}
      </div>
      {#if settingsState.saveError}
        <div class="error-banner">{settingsState.saveError}</div>
      {/if}
    {/if}
  </div>
</div>

<style>
  .settings-container {
    padding: 2rem;
    max-width: 800px;
    margin: 0 auto;
  }

  .header {
    margin-bottom: 2rem;
  }

  h2 {
    font-size: 1.5rem;
    font-weight: 600;
    margin: 0 0 0.5rem 0;
    color: #f3f4f6;
  }

  .header p {
    color: #9ca3af;
    margin: 0;
  }

  .settings-content {
    display: flex;
    flex-direction: column;
    gap: 2rem;
  }

  .error-message {
    background: rgba(220, 38, 38, 0.15);
    border: 1px solid rgba(220, 38, 38, 0.4);
    color: #fca5a5;
    padding: 0.75rem 1rem;
    border-radius: 0.5rem;
    font-size: 0.8125rem;
  }

  .error-banner {
    position: fixed;
    bottom: 1rem;
    right: 1rem;
    background: #7f1d1d;
    border: 1px solid #dc2626;
    color: #fca5a5;
    padding: 0.75rem 1.25rem;
    border-radius: 0.5rem;
    font-size: 0.8125rem;
    z-index: 100;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4);
    max-width: 400px;
  }

  .setting-group {
    background-color: #111827;
    border-radius: 0.75rem;
  }

  .setting-group h3 {
    font-size: 1.125rem;
    color: #f3f4f6;
    margin: 0 0 1rem 0;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    border-bottom: 1px solid #374151;
    padding-bottom: 0.75rem;
  }

  .setting-field {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    margin-bottom: 1.5rem;
  }

  .split-fields {
    display: flex;
    gap: 1.5rem;
  }

  .split-fields .setting-field {
    flex: 1;
    margin-bottom: 0;
  }

  label {
    font-size: 0.875rem;
    font-weight: 500;
    color: #e5e7eb;
    display: flex;
    align-items: center;
  }

  input[type="text"],
  input[type="password"],
  input[type="number"],
  select {
    background-color: #1f2937;
    border: 1px solid #374151;
    color: #f3f4f6;
    border-radius: 0.5rem;
    padding: 0.75rem 1rem;
    font-size: 0.9375rem;
    outline: none;
    transition: border-color 0.2s;
  }

  input:focus,
  select:focus {
    border-color: #3b82f6;
  }

  input:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .help-text {
    font-size: 0.75rem;
    color: #6b7280;
    margin: 0;
  }

  .mt-n2 {
    margin-top: -0.5rem;
  }

  .loading {
    color: #9ca3af;
    text-align: center;
    padding: 2rem;
  }

  .keyring-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .keyring-row input {
    flex: 1;
  }

  .keyring-status {
    font-size: 0.875rem;
    color: #34d399;
    padding: 0.75rem 0;
  }

  .btn {
    background-color: #3b82f6;
    color: white;
    border: none;
    border-radius: 0.5rem;
    padding: 0.75rem 1rem;
    font-size: 0.875rem;
    font-weight: 500;
    cursor: pointer;
    transition: background-color 0.2s;
    white-space: nowrap;
  }

  .btn:hover {
    background-color: #2563eb;
  }

  .btn-small {
    padding: 0.625rem 0.875rem;
    font-size: 0.8125rem;
  }

  .btn-danger {
    background-color: #dc2626;
  }

  .btn-danger:hover {
    background-color: #b91c1c;
  }

  .success-text {
    color: #34d399;
    font-size: 0.8125rem;
    margin: 0.25rem 0 0 0;
  }
</style>
