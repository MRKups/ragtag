<script lang="ts">
  import { settingsState } from '$lib/stores/settings.svelte';
  import { Cpu, Database, Link, Key, BrainCircuit } from 'lucide-svelte';
  import Toggle from './Toggle.svelte';

  const modes = [
    { value: 'local', label: 'Local (In-process)' },
    { value: 'external', label: 'External (OpenAI-compatible)' }
  ];

  const isqLevels = [
    { value: 'Q4K', label: 'Q4K (Recommended)' },
    { value: 'Q8_0', label: 'Q8_0' },
    { value: 'none', label: 'None (FP16)' }
  ];

  function updateSetting(key: string, value: string) {
    settingsState.update(key, value);
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
              onblur={() => updateSetting('external_base_url', settingsState.external_base_url)}
            />
          </div>

          <div class="setting-field">
            <label for="external_api_key">
              <Key size={14} class="mr-1 inline" />
              API Key
            </label>
            <input
              id="external_api_key"
              type="password"
              placeholder="sk-..."
              value="********"
              disabled
              title="Stored securely in OS keychain. Manage via backend command."
            />
            <p class="help-text">Stored securely in the OS keychain. Pending task 54.</p>
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
              placeholder="google/gemma-2-2b-it-GGUF"
              bind:value={settingsState.generation_model}
              onblur={() => updateSetting('generation_model', settingsState.generation_model)}
            />
          </div>

          <div class="setting-field">
            <label for="isq_level">In-Situ Quantization</label>
            <select
              id="isq_level"
              bind:value={settingsState.isq_level}
              onchange={() => updateSetting('isq_level', settingsState.isq_level)}
            >
              {#each isqLevels as level}
                <option value={level.value}>{level.label}</option>
              {/each}
            </select>
            <p class="help-text">Lowers RAM usage. Requires restart to apply.</p>
          </div>
        {/if}
      </div>

      <div class="setting-group">
        <h3><Database size={18} /> RAG Pipeline Configuration</h3>

        <Toggle
          bind:checked={settingsState.use_query_enhancement}
          label="Query Enhancement"
          description="Automatically expand queries with technical synonyms to improve search recall."
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
</style>
