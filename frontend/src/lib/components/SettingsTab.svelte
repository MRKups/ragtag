<script lang="ts">
  import { onMount } from 'svelte';
  import { DEFAULT_LOCAL_INFERENCE, settingsState } from '$lib/stores/settings.svelte';
  import { api } from '$lib/api';
  import { Cpu, Database, HardDrive, Link, Key, BrainCircuit, Cloud, RotateCcw } from '@lucide/svelte';
  import { errorDialog } from '$lib/stores/error-dialog.svelte';
  import Toggle from './Toggle.svelte';
  import AppSelect, { type SelectOption } from './AppSelect.svelte';

  type InferenceConfig = 'local' | 'external';


  let apiKeyValue = $state('');
  let apiKeyExists = $state(false);
  let apiKeySaved = $state(false);

  const protocolOptions: SelectOption[] = [
    { value: 'http', label: 'HTTP' },
    { value: 'https', label: 'HTTPS' }
  ];

  let configuringInference = $state<InferenceConfig | null>(null);
  let isRestoringLocalDefaults = $state(false);
  let externalProtocolDraft = $state('http');
  let externalServerAddressDraft = $state('');
  let externalPortDraft = $state('');
  let externalModelDraft = $state('');
  let availableExternalModels = $state<string[]>([]);
  let externalConnectionStatus = $state('');
  let isTestingExternalProvider = $state(false);
  let isSavingExternalProvider = $state(false);
  const pendingSettingUpdates = new Map<string, Promise<boolean>>();

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
    } catch {
      errorDialog.report({
        dedupeKey: 'api-key-status',
        title: 'Keychain unavailable',
        message: 'Ragtag could not check the remote provider API key.',
        guidance: 'Open remote settings and try again after the system keychain is available.'
      });
      console.error('Failed to check API key');
    }
    await loadStorageInfo();
  });

  $effect(() => {
    const requested = settingsState.requestedInferenceConfiguration;
    if (requested) {
      openInferenceConfiguration(settingsState.consumeInferenceConfiguration()!);
    }
  });

  function resetExternalModelDiscovery() {
    externalModelDraft = '';
    availableExternalModels = [];
    externalConnectionStatus = '';
  }

  function loadExternalEndpointDraft(value: string) {
    try {
      const endpoint = new URL(value.trim() || 'http://localhost:11434/v1');
      externalProtocolDraft = endpoint.protocol === 'https:' ? 'https' : 'http';
      externalServerAddressDraft = endpoint.hostname;
      externalPortDraft = endpoint.port;
    } catch {
      externalProtocolDraft = 'http';
      externalServerAddressDraft = 'localhost';
      externalPortDraft = '11434';
    }
  }

  function buildExternalBaseUrl(): string | null {
    const serverAddress = externalServerAddressDraft.trim();
    const port = externalPortDraft.trim();
    const isBracketedIpv6 = serverAddress.startsWith('[') && serverAddress.endsWith(']');
    if (
      !serverAddress
      || serverAddress.includes('://')
      || serverAddress.includes('/')
      || serverAddress.includes('@')
      || serverAddress.includes('?')
      || serverAddress.includes('#')
      || (!isBracketedIpv6 && serverAddress.includes(':'))
    ) {
      errorDialog.report({
        dedupeKey: 'external-provider-address',
        title: 'Server address is invalid',
        message: 'Enter a hostname or IP address only, without a protocol, path, or port.',
        guidance: 'For example: localhost, 192.168.1.50, or [fd00::1].'
      });
      return null;
    }
    if (port && (!/^\d+$/.test(port) || Number(port) < 1 || Number(port) > 65535)) {
      errorDialog.report({
        dedupeKey: 'external-provider-port',
        title: 'Port is invalid',
        message: 'Enter a whole port number between 1 and 65535.',
        guidance: 'Leave the port blank only when the provider uses the default HTTP or HTTPS port.'
      });
      return null;
    }

    const authority = `${serverAddress}${port ? `:${port}` : ''}`;
    return new URL(`${externalProtocolDraft}://${authority}/v1`).toString().replace(/\/$/, '');
  }

  function openInferenceConfiguration(mode: InferenceConfig) {
    configuringInference = mode;
    if (mode === 'external') {
      loadExternalEndpointDraft(settingsState.external_base_url);
      externalModelDraft = settingsState.external_model;
      availableExternalModels = [];
      externalConnectionStatus = '';
    }
  }

  function updateSetting(key: string, value: string): Promise<boolean> {
    const previous = pendingSettingUpdates.get(key);
    const pending = (async () => {
      if (previous) await previous;
      return settingsState.update(key, value);
    })();
    pendingSettingUpdates.set(key, pending);
    void pending.finally(() => {
      if (pendingSettingUpdates.get(key) === pending) pendingSettingUpdates.delete(key);
    });
    return pending;
  }

  async function restoreLocalDefaults() {
    isRestoringLocalDefaults = true;
    try {
      await Promise.all(
        Object.keys(DEFAULT_LOCAL_INFERENCE).map(
          (key) => pendingSettingUpdates.get(key) ?? Promise.resolve(true)
        )
      );
      await Promise.all(
        Object.entries(DEFAULT_LOCAL_INFERENCE).map(([key, value]) => updateSetting(key, value))
      );
    } finally {
      isRestoringLocalDefaults = false;
    }
  }

  async function setDefaultInference(mode: InferenceConfig) {
    if (mode === settingsState.inference_mode) return;
    await updateSetting('inference_mode', mode);
  }

  function handleInferenceSelectionKeydown(event: KeyboardEvent, mode: InferenceConfig) {
    let nextMode: InferenceConfig | null = null;

    if (event.key === 'Home') nextMode = 'local';
    else if (event.key === 'End') nextMode = 'external';
    else if (['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) {
      nextMode = mode === 'local' ? 'external' : 'local';
    }

    if (!nextMode) return;

    event.preventDefault();
    void setDefaultInference(nextMode);
    document.querySelector<HTMLButtonElement>(`[data-inference-mode="${nextMode}"]`)?.focus();
  }

  async function saveApiKey() {
    if (!apiKeyValue.trim()) return;
    try {
      await api.setApiKey(apiKeyValue.trim());
      apiKeyExists = true;
      apiKeySaved = true;
      apiKeyValue = '';
      setTimeout(() => (apiKeySaved = false), 2000);
    } catch {
      errorDialog.report({
        dedupeKey: 'api-key-save',
        title: 'API key not saved',
        message: 'Ragtag could not save the API key in your system keychain.',
        guidance: 'Check that your system keychain is available, then try again.'
      });
      console.error('Failed to save API key');
    }
  }

  async function clearApiKey() {
    try {
      await api.deleteApiKey();
      apiKeyExists = false;
      apiKeyValue = '';
    } catch {
      errorDialog.report({
        dedupeKey: 'api-key-delete',
        title: 'API key not removed',
        message: 'Ragtag could not remove the API key from your system keychain.',
        guidance: 'Try again after the system keychain is available.'
      });
      console.error('Failed to delete API key');
    }
  }

  async function testExternalProvider() {
    const baseUrl = buildExternalBaseUrl();
    if (!baseUrl) return;

    isTestingExternalProvider = true;
    externalConnectionStatus = '';
    try {
      const connection = await api.testExternalProvider(baseUrl, apiKeyValue.trim() || null);
      availableExternalModels = connection.models;
      if (!availableExternalModels.includes(externalModelDraft)) {
        externalModelDraft = '';
      }
      externalConnectionStatus = `Connected. ${connection.models.length} model${connection.models.length === 1 ? '' : 's'} available.`;
    } catch {
      externalConnectionStatus = 'Connection test failed. Review the error dialog and try again.';
      errorDialog.report({
        dedupeKey: 'external-provider-test',
        title: 'Remote provider unavailable',
        message: 'Ragtag could not connect to that remote provider.',
        guidance: 'Verify the base URL, start the provider, and save an API key if it requires authentication.'
      });
    } finally {
      isTestingExternalProvider = false;
    }
  }

  async function saveExternalProvider() {
    const baseUrl = buildExternalBaseUrl();
    const model = externalModelDraft.trim();
    if (!baseUrl || !model || !availableExternalModels.includes(model)) {
      errorDialog.report({
        dedupeKey: 'external-provider-save-input',
        title: 'Provider configuration incomplete',
        message: 'Test the provider, then select one of its available models before saving.',
        guidance: 'Saving keeps the existing provider configuration unchanged until both fields are valid.'
      });
      return;
    }

    isSavingExternalProvider = true;
    try {
      await api.setExternalProviderConfig(baseUrl, model);
      settingsState.external_base_url = baseUrl;
      settingsState.external_model = model;
      externalConnectionStatus = 'Provider saved. Ragtag is validating it for queries.';
    } catch {
      externalConnectionStatus = 'Provider was not saved. Review the error dialog and try again.';
      errorDialog.report({
        dedupeKey: 'external-provider-save',
        title: 'Provider configuration was not saved',
        message: 'Ragtag could not save the tested remote provider configuration.',
        guidance: 'Your previous provider configuration remains in effect. Try again.'
      });
    } finally {
      isSavingExternalProvider = false;
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
    } catch {
      errorDialog.report({
        dedupeKey: 'storage-info',
        title: 'Storage information unavailable',
        message: 'Ragtag could not read its local storage information.',
        guidance: 'Try again after checking that Ragtag can access its local data directory.'
      });
      console.error('Failed to load storage info');
    }
  }

  async function doCompact() {
    try {
      await api.compactVectorStore();
      storageMessage = 'Vector store compacted.';
      await loadStorageInfo();
    } catch {
      storageMessage = '';
      errorDialog.report({
        dedupeKey: 'vector-store-compact',
        title: 'Vector store was not compacted',
        message: 'Ragtag could not compact the local vector store.',
        guidance: 'Try again. Your indexed documents remain available.'
      });
    }
    if (storageMessage) setTimeout(() => (storageMessage = ''), 3000);
  }

  async function doVacuum() {
    try {
      await api.vacuumDatabase();
      storageMessage = 'Database vacuumed.';
      await loadStorageInfo();
    } catch {
      storageMessage = '';
      errorDialog.report({
        dedupeKey: 'database-vacuum',
        title: 'Database was not vacuumed',
        message: 'Ragtag could not optimize the local database.',
        guidance: 'Try again. Your documents and settings remain unchanged.'
      });
    }
    if (storageMessage) setTimeout(() => (storageMessage = ''), 3000);
  }

  async function doReset() {
    try {
      storageMessage = 'Restarting to reset local data...';
      await api.resetAllData(resetDeleteModels);
    } catch {
      storageMessage = '';
      errorDialog.report({
        dedupeKey: 'reset-data',
        title: 'Data reset did not start',
        message: 'Ragtag could not start the local data reset.',
        guidance: 'Your documents and settings remain unchanged. Try again.'
      });
    }
  }
</script>

<div class="settings-container">
  <header class="page-header">
    <p class="eyebrow">Preferences</p>
    <h2>Settings</h2>
    <p class="page-intro">Configure how RAGtag runs, retrieves context, and manages local storage.</p>
  </header>

  <div class="settings-content">
    {#if settingsState.isLoading}
      <div class="loading" role="status">Loading settings...</div>
    {:else}

      <section class="setting-group" aria-labelledby="inference-heading">
        <div class="section-heading inference-heading">
          <div class="section-icon"><Cpu size={18} /></div>
          <div class="inference-heading-content">
            <div>
              <h3 id="inference-heading">Inference configuration</h3>
              <p>Select and configure each source independently.</p>
            </div>
            <div class="inference-status" role="status">
              <span class="inference-status-label">Active inference</span>
              <strong>
                {#if settingsState.inference_mode === 'local'}
                  <HardDrive size={14} aria-hidden="true" /> Local model
                {:else}
                  <Cloud size={14} aria-hidden="true" /> Remote provider
                {/if}
              </strong>
            </div>
          </div>
        </div>

        <div class="inference-picker" role="radiogroup" aria-labelledby="inference-heading">
          <article class="inference-card" class:active={settingsState.inference_mode === 'local'}>
            <button
              type="button"
              class="inference-card-select"
              role="radio"
              aria-checked={settingsState.inference_mode === 'local'}
              tabindex={settingsState.inference_mode === 'local' ? 0 : -1}
              data-inference-mode="local"
              onclick={() => setDefaultInference('local')}
              onkeydown={(event) => handleInferenceSelectionKeydown(event, 'local')}
            >
              <div class="inference-card-icon"><HardDrive size={18} /></div>
              <div class="inference-card-copy">
                <span>Local model</span>
                <small>On this device</small>
              </div>
            </button>
            <button type="button" class="btn btn-small btn-secondary configure-btn" onclick={() => openInferenceConfiguration('local')}>
              Configure
            </button>
          </article>

          <article class="inference-card" class:active={settingsState.inference_mode === 'external'}>
            <button
              type="button"
              class="inference-card-select"
              role="radio"
              aria-checked={settingsState.inference_mode === 'external'}
              tabindex={settingsState.inference_mode === 'external' ? 0 : -1}
              data-inference-mode="external"
              onclick={() => setDefaultInference('external')}
              onkeydown={(event) => handleInferenceSelectionKeydown(event, 'external')}
            >
              <div class="inference-card-icon"><Cloud size={18} /></div>
              <div class="inference-card-copy">
                <span>Remote provider</span>
                <small>OpenAI-compatible endpoint</small>
              </div>
            </button>
            <button type="button" class="btn btn-small btn-secondary configure-btn" onclick={() => openInferenceConfiguration('external')}>
              Configure
            </button>
          </article>
        </div>

        {#if configuringInference}
          <div class="config-modal-overlay">
            <button type="button" class="config-modal-backdrop" aria-label="Close inference configuration" onclick={() => (configuringInference = null)}></button>
            <div
              class="config-modal"
              role="dialog"
              aria-modal="true"
              aria-labelledby="config-modal-title"
              tabindex="-1"
              onkeydown={(event) => event.key === 'Escape' && (configuringInference = null)}
            >
              {#if configuringInference === 'local'}
                <p class="panel-eyebrow">Local inference</p>
                <h4 id="config-modal-title">Local model configuration</h4>
                <p class="config-modal-intro">Configure the GGUF model and tokenizer used for private, on-device generation.</p>

                <div class="inference-fields">
                  <div class="setting-field">
                    <label for="generation_model">
                      <BrainCircuit size={14} class="field-label-icon" />
                      Hugging Face model repo
                    </label>
                    <input
                      id="generation_model"
                      type="text"
                      placeholder={DEFAULT_LOCAL_INFERENCE.generation_model}
                      bind:value={settingsState.generation_model}
                      onblur={() => { const value = settingsState.generation_model.trim(); if (value) updateSetting('generation_model', value); }}
                    />
                    <p class="help-text">GGUF models are pre-quantized — ISQ does not apply in local mode.</p>
                  </div>

                  <div class="setting-field">
                    <label for="tokenizer_model">
                      <BrainCircuit size={14} class="field-label-icon" />
                      Tokenizer model
                    </label>
                    <input
                      id="tokenizer_model"
                      type="text"
                      placeholder={DEFAULT_LOCAL_INFERENCE.tokenizer_model}
                      bind:value={settingsState.tokenizer_model}
                      onblur={() => { const value = settingsState.tokenizer_model.trim(); if (value) updateSetting('tokenizer_model', value); }}
                    />
                    <p class="help-text">Use a tokenizer repository compatible with the selected GGUF model.</p>
                  </div>

                  <div class="setting-field">
                    <label for="gguf_filename">GGUF filename</label>
                    <input
                      id="gguf_filename"
                      type="text"
                      placeholder={DEFAULT_LOCAL_INFERENCE.gguf_filename}
                      bind:value={settingsState.gguf_filename}
                      onblur={() => { const value = settingsState.gguf_filename.trim(); if (value) updateSetting('gguf_filename', value); }}
                    />
                    <p class="help-text">Filename of the GGUF file within the repository above.</p>
                  </div>
                </div>

                <div class="restore-defaults">
                  <button
                    type="button"
                    class="btn btn-small btn-secondary"
                    disabled={isRestoringLocalDefaults}
                    onclick={restoreLocalDefaults}
                  >
                    <RotateCcw size={14} />
                    {isRestoringLocalDefaults ? 'Restoring...' : 'Restore local defaults'}
                  </button>
                  <p class="help-text">Restores Ragtag’s recommended Qwen model. Restart to apply model changes.</p>
                </div>
              {:else}
                <p class="panel-eyebrow">Remote inference</p>
                <h4 id="config-modal-title">Remote provider configuration</h4>
                <p class="config-modal-intro">Configure an OpenAI-compatible provider independently from the default selection.</p>

                <div class="inference-fields">
                  <div class="provider-endpoint-fields">
                    <div class="setting-field">
                      <span class="field-label">Protocol</span>
                      <AppSelect
                        id="external_protocol"
                        options={protocolOptions}
                        bind:value={externalProtocolDraft}
                        placeholder="Protocol"
                        ariaLabel="Provider protocol"
                        onvaluechange={resetExternalModelDiscovery}
                      />
                    </div>

                    <div class="setting-field">
                      <label for="external_server_address">
                        <Link size={14} class="field-label-icon" />
                        Server address
                      </label>
                      <input
                        id="external_server_address"
                        type="text"
                        placeholder="192.168.1.50"
                        bind:value={externalServerAddressDraft}
                        oninput={resetExternalModelDiscovery}
                      />
                    </div>

                    <div class="setting-field">
                      <label for="external_port">Port <span>(optional)</span></label>
                      <input
                        id="external_port"
                        type="text"
                        inputmode="numeric"
                        maxlength="5"
                        placeholder="1234"
                        bind:value={externalPortDraft}
                        oninput={resetExternalModelDiscovery}
                      />
                    </div>
                  </div>
                  <p class="help-text">Ragtag uses the conventional OpenAI-compatible <code>/v1</code> API path automatically.</p>

                  <div class="setting-field">
                    <span class="field-label">
                      <BrainCircuit size={14} class="field-label-icon" />
                      External model
                    </span>
                    <AppSelect
                      id="external_model"
                      options={availableExternalModels.map((model) => ({ value: model, label: model }))}
                      bind:value={externalModelDraft}
                      placeholder="Select a model"
                      disabled={availableExternalModels.length === 0 || isTestingExternalProvider}
                      ariaLabel="External model"
                    />
                    <p class="help-text">
                      {availableExternalModels.length === 0
                        ? 'Test the connection to load models from this provider.'
                        : 'Select the model Ragtag should use for remote answers.'}
                    </p>
                  </div>

                  <div class="setting-field">
                    <span class="field-label">
                      <Key size={14} class="field-label-icon" />
                      API key
                    </span>
                    {#if apiKeyExists}
                      <div class="keyring-row">
                        <span class="keyring-status">Stored in OS keychain</span>
                        <button type="button" class="btn btn-small btn-danger" onclick={clearApiKey}>Clear key</button>
                      </div>
                    {:else}
                      <div class="keyring-row">
                        <input
                          id="external_api_key"
                          type="password"
                          placeholder="sk-..."
                          autocomplete="off"
                          aria-label="External API key"
                          bind:value={apiKeyValue}
                          onkeydown={(event) => event.key === 'Enter' && saveApiKey()}
                        />
                        <button type="button" class="btn btn-small" onclick={saveApiKey}>Save key</button>
                      </div>
                    {/if}
                    {#if apiKeySaved}
                      <p class="success-text" role="status">API key saved securely.</p>
                    {/if}
                    <p class="help-text">Stored in the OS keychain, never in the database or on disk. Save it before testing when required by the provider.</p>
                  </div>
                </div>

                {#if externalConnectionStatus}
                  <p class="provider-connection-status" role="status">{externalConnectionStatus}</p>
                {/if}
              {/if}

              <div class="config-modal-actions">
                {#if configuringInference === 'external'}
                  <button
                    type="button"
                    class="btn btn-secondary"
                    disabled={isTestingExternalProvider || isSavingExternalProvider || !externalServerAddressDraft.trim()}
                    onclick={testExternalProvider}
                  >
                    {isTestingExternalProvider ? 'Testing connection…' : 'Test connection'}
                  </button>
                  <button
                    type="button"
                    class="btn"
                    disabled={isTestingExternalProvider || isSavingExternalProvider || !externalServerAddressDraft.trim() || !externalModelDraft.trim() || !availableExternalModels.includes(externalModelDraft.trim())}
                    onclick={saveExternalProvider}
                  >
                    {isSavingExternalProvider ? 'Saving provider…' : 'Save provider'}
                  </button>
                {/if}
                <button type="button" class="btn btn-secondary" onclick={() => (configuringInference = null)}>Done</button>
              </div>
            </div>
          </div>
        {/if}
      </section>

      <section class="setting-group" aria-labelledby="pipeline-heading">
        <div class="section-heading">
          <div class="section-icon"><Database size={18} /></div>
          <div>
            <h3 id="pipeline-heading">RAG pipeline</h3>
            <p>Fine-tune retrieval behavior for your documents.</p>
          </div>
        </div>

        <Toggle
          bind:checked={settingsState.use_query_enhancement}
          label="Query Enhancement"
          description="Expand recognized technical abbreviations to improve search recall."
          onchange={() => updateSetting('use_query_enhancement', String(settingsState.use_query_enhancement))}
        />

        <div class="pipeline-fields">
          <div class="setting-field">
            <label for="chunk_size">Chunk size <span>(estimated tokens)</span></label>
            <input
              id="chunk_size"
              type="number"
              bind:value={settingsState.chunk_size}
              onblur={() => updateSetting('chunk_size', settingsState.chunk_size.toString())}
            />
          </div>
          <div class="setting-field">
            <label for="chunk_overlap">Chunk overlap <span>(estimated tokens)</span></label>
            <input
              id="chunk_overlap"
              type="number"
              bind:value={settingsState.chunk_overlap}
              onblur={() => updateSetting('chunk_overlap', settingsState.chunk_overlap.toString())}
            />
          </div>
        </div>
        <p class="help-text pipeline-note">Each estimated token is about 4 characters; overlap is capped at half of the chunk size. Changing either setting reindexes existing indexed documents.</p>
      </section>

      <section class="setting-group" aria-labelledby="storage-heading">
        <div class="section-heading">
          <div class="section-icon"><Database size={18} /></div>
          <div>
            <h3 id="storage-heading">Storage management</h3>
            <p>Review local usage and perform routine maintenance when needed.</p>
          </div>
        </div>

        {#if storageInfo}
          <dl class="storage-metrics">
            <div class="metric-card">
              <dt>Database</dt>
              <dd>{formatBytes(storageInfo.database_bytes)}</dd>
            </div>
            <div class="metric-card">
              <dt>Vector store</dt>
              <dd>{formatBytes(storageInfo.vector_store_bytes)}</dd>
            </div>
            <div class="metric-card">
              <dt>Documents</dt>
              <dd>{formatBytes(storageInfo.documents_bytes)}</dd>
            </div>
            <div class="metric-card">
              <dt>Models</dt>
              <dd>{formatBytes(storageInfo.models_bytes)}</dd>
            </div>
          </dl>
        {/if}

        <div class="maintenance-panel">
          <div class="maintenance-copy">
            <h4>Routine maintenance</h4>
            <p>Refresh the usage snapshot or reclaim unused database and vector-store space.</p>
          </div>
          <div class="storage-actions">
            <button type="button" class="btn btn-secondary" onclick={loadStorageInfo}>
              <Database size={14} /> Refresh usage
            </button>
            <button type="button" class="btn btn-secondary" onclick={doCompact}>Compact vector store</button>
            <button type="button" class="btn btn-secondary" onclick={doVacuum}>VACUUM database</button>
          </div>
        </div>

        <aside class="danger-zone" aria-labelledby="reset-heading">
          <div>
            <h4 id="reset-heading">Reset application data</h4>
            <p>Permanently remove documents, vectors, and the database from this device.</p>
          </div>
          <button type="button" class="btn btn-danger" onclick={() => (showResetConfirm = true)}>
            Reset all data
          </button>
        </aside>

        {#if showResetConfirm}
          <div class="reset-modal-overlay">
            <button type="button" class="reset-modal-backdrop" aria-label="Close reset confirmation" onclick={() => (showResetConfirm = false)}></button>
            <div
              class="reset-modal"
              role="dialog"
              aria-modal="true"
              aria-labelledby="reset-modal-title"
              aria-describedby="reset-modal-description"
              tabindex="-1"
              onkeydown={(e) => e.key === 'Escape' && (showResetConfirm = false)}
            >
              <p class="modal-eyebrow">Destructive action</p>
              <h4 id="reset-modal-title">Reset all data?</h4>
              <p id="reset-modal-description">This permanently deletes all documents, vectors, and the database. The app must restart to apply the reset.</p>
              <label class="checkbox-label">
                <input type="checkbox" bind:checked={resetDeleteModels} />
                <span>Also delete app-managed local model downloads</span>
              </label>
              <div class="reset-modal-actions">
                <button type="button" class="btn btn-secondary" onclick={() => (showResetConfirm = false)}>Cancel</button>
                <button type="button" class="btn btn-danger" onclick={doReset}>Reset &amp; restart</button>
              </div>
            </div>
          </div>
        {/if}

        {#if storageMessage}
          <p class="success-text storage-status" role="status">{storageMessage}</p>
        {/if}
      </section>
    {/if}
  </div>
</div>

<style>
  .settings-container {
    --page-bg: #111827;
    --surface: #1f2937;
    --surface-raised: #273449;
    --surface-subtle: rgba(15, 23, 42, 0.48);
    --surface-soft: rgba(31, 41, 55, 0.5);
    --border: #374151;
    --border-strong: #4b5563;
    --text-strong: #f3f4f6;
    --text: #e5e7eb;
    --text-muted: #9ca3af;
    --text-subtle: #6b7280;
    --primary: #2563eb;
    --primary-hover: #1d4ed8;
    --primary-ring: #3b82f6;
    --primary-soft: rgba(37, 99, 235, 0.15);
    --primary-border: rgba(59, 130, 246, 0.3);
    --focus-ring: rgba(59, 130, 246, 0.2);
    --focus-ring-strong: rgba(59, 130, 246, 0.28);
    --danger: #dc2626;
    --danger-hover: #b91c1c;
    --danger-soft: rgba(127, 29, 29, 0.22);
    --danger-border: rgba(220, 38, 38, 0.4);
    --danger-border-strong: rgba(220, 38, 38, 0.45);
    --danger-text: #fecaca;
    --danger-text-muted: #fca5a5;
    --danger-banner: #7f1d1d;
    --success: #34d399;
    --success-soft: rgba(6, 95, 70, 0.22);
    --success-border: rgba(52, 211, 153, 0.3);
    --overlay: rgba(2, 6, 23, 0.72);
    --radius-sm: 0.5rem;
    --radius-md: 0.75rem;
    --radius-lg: 1rem;
    --shadow-raised: 0 14px 30px rgba(0, 0, 0, 0.16);
    --shadow-modal: 0 24px 60px rgba(0, 0, 0, 0.42);
    width: min(100%, 52rem);
    margin: 0 auto;
    padding: 2.5rem 2rem 3rem;
  }

  .page-header {
    margin-bottom: 2rem;
    padding: 0 0.25rem;
  }

  .eyebrow,
  .modal-eyebrow {
    margin: 0 0 0.375rem;
    color: var(--primary-ring);
    font-size: 0.6875rem;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
  }

  h2,
  h3,
  h4,
  p {
    margin-top: 0;
  }

  h2 {
    margin-bottom: 0.5rem;
    color: var(--text-strong);
    font-size: 1.75rem;
    font-weight: 650;
    letter-spacing: -0.025em;
  }

  .page-intro {
    max-width: 38rem;
    margin-bottom: 0;
    color: var(--text-muted);
    font-size: 0.9375rem;
    line-height: 1.55;
  }

  .settings-content {
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
  }

  .setting-group {
    padding: 1.5rem;
    background: var(--surface-subtle);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-raised);
  }

  .section-heading {
    display: flex;
    align-items: flex-start;
    gap: 0.75rem;
    margin-bottom: 1.5rem;
    padding-bottom: 1.125rem;
    border-bottom: 1px solid var(--border);
  }

  .section-icon {
    display: grid;
    flex: 0 0 auto;
    width: 2rem;
    height: 2rem;
    place-items: center;
    color: var(--primary-ring);
    background: var(--primary-soft);
    border: 1px solid var(--primary-border);
    border-radius: var(--radius-sm);
  }

  .section-heading h3 {
    margin-bottom: 0.1875rem;
    color: var(--text-strong);
    font-size: 1rem;
    font-weight: 650;
    letter-spacing: -0.01em;
  }

  .section-heading p {
    margin-bottom: 0;
    color: var(--text-muted);
    font-size: 0.8125rem;
    line-height: 1.45;
  }

  .inference-heading {
    align-items: center;
  }

  .inference-heading-content {
    display: flex;
    flex: 1;
    align-items: center;
    justify-content: space-between;
    min-width: 0;
    gap: 1rem;
  }

  .inference-status {
    display: flex;
    flex: 0 0 auto;
    align-items: center;
    gap: 0.5rem;
    padding: 0.5rem 0.625rem;
    background: var(--page-bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
  }

  .inference-status-label {
    color: var(--text-subtle);
    font-size: 0.625rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .inference-status strong {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    color: var(--primary-ring);
    font-size: 0.8125rem;
    font-weight: 650;
  }

  .pipeline-fields {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 1rem;
  }

  .setting-field {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    min-width: 0;
  }


  .inference-picker {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 0.75rem;
    margin-bottom: 1rem;
  }

  .inference-card {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    min-width: 0;
    min-height: 5rem;
    padding: 0.5rem;
    background: var(--page-bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    transition: border-color 160ms ease-out, background-color 160ms ease-out, box-shadow 160ms ease-out;
  }

  .inference-card:hover {
    border-color: var(--border-strong);
    background: var(--surface-soft);
  }

  .inference-card.active {
    background: var(--primary-soft);
    border-color: var(--primary-ring);
    box-shadow: 0 0 0 3px var(--focus-ring);
  }

  .inference-card-select {
    display: flex;
    align-items: center;
    flex: 1;
    min-width: 0;
    min-height: 4rem;
    padding: 0.375rem;
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: pointer;
    background: transparent;
    border: 0;
    border-radius: var(--radius-sm);
  }

  .inference-card-select:focus-visible {
    outline: none;
    box-shadow: 0 0 0 3px var(--focus-ring-strong);
  }

  .inference-card-icon {
    display: grid;
    flex: 0 0 auto;
    width: 2.25rem;
    height: 2.25rem;
    margin-right: 0.75rem;
    place-items: center;
    color: var(--primary-ring);
    background: var(--primary-soft);
    border: 1px solid var(--primary-border);
    border-radius: var(--radius-sm);
  }

  .inference-card-copy {
    display: flex;
    flex-direction: column;
    min-width: 0;
    gap: 0.1875rem;
  }

  .inference-card-copy > span {
    color: var(--text-strong);
    font-size: 0.875rem;
    font-weight: 650;
  }

  .inference-card-copy small {
    overflow: hidden;
    color: var(--text-subtle);
    font-size: 0.75rem;
    text-overflow: ellipsis;
    white-space: nowrap;
  }


  .configure-btn {
    flex: 0 0 auto;
  }

  .panel-eyebrow {
    margin: 0 0 0.25rem;
    color: var(--primary-ring);
    font-size: 0.625rem;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
  }

  .inference-fields {
    display: grid;
    gap: 1rem;
  }
  .provider-endpoint-fields {
    display: grid;
    grid-template-columns: minmax(6.5rem, 0.8fr) minmax(0, 2fr) minmax(5.5rem, 0.7fr);
    gap: 0.75rem;
  }

  .provider-endpoint-fields label span {
    color: var(--text-subtle);
    font-weight: 500;
  }


  .field-label {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    color: var(--text);
    font-size: 0.8125rem;
    font-weight: 600;
  }

  .restore-defaults {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin-top: 1.25rem;
    padding-top: 1.25rem;
    border-top: 1px solid var(--border);
  }

  .restore-defaults .help-text {
    max-width: 24rem;
  }

  .config-modal-overlay {
    position: fixed;
    inset: 0;
    z-index: 90;
    display: grid;
    place-items: center;
    padding: 1rem;
    background: var(--overlay);
  }

  .config-modal-backdrop {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    padding: 0;
    cursor: default;
    background: transparent;
    border: 0;
  }

  .config-modal {
    position: relative;
    z-index: 1;
    width: min(100%, 38rem);
    max-height: calc(100vh - 2rem);
    padding: 1.5rem;
    overflow-y: auto;
    background: var(--surface);
    border: 1px solid var(--primary-border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-modal);
  }

  .config-modal h4 {
    margin-bottom: 0.5rem;
    color: var(--text-strong);
    font-size: 1.25rem;
    font-weight: 650;
    letter-spacing: -0.02em;
  }

  .config-modal-intro {
    margin-bottom: 1.25rem;
    color: var(--text-muted);
    font-size: 0.875rem;
    line-height: 1.55;
  }

  .config-modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.75rem;
    margin-top: 1.25rem;
    padding-top: 1.25rem;
    border-top: 1px solid var(--border);
  }

  label {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    color: var(--text);
    font-size: 0.8125rem;
    font-weight: 600;
  }

  .pipeline-fields label span {
    color: var(--text-subtle);
    font-weight: 500;
  }

  :global(.field-label-icon) {
    color: var(--text-muted);
  }

  input[type="text"],
  input[type="password"],
  input[type="number"] {
    width: 100%;
    min-height: 2.75rem;
    padding: 0.625rem 0.75rem;
    color: var(--text-strong);
    font: inherit;
    font-size: 0.875rem;
    line-height: 1.25;
    background: var(--page-bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    outline: none;
    transition: border-color 160ms ease-out, box-shadow 160ms ease-out, background-color 160ms ease-out;
  }

  input::placeholder {
    color: var(--text-subtle);
  }

  input:hover {
    border-color: var(--border-strong);
  }

  input:focus {
    border-color: var(--primary-ring);
    box-shadow: 0 0 0 3px var(--focus-ring);
  }

  input:disabled {
    color: var(--text-subtle);
    cursor: not-allowed;
    opacity: 0.65;
  }

  .provider-connection-status {
    margin: 1rem 0 0;
    padding: 0.625rem 0.75rem;
    color: var(--success);
    font-size: 0.8125rem;
    line-height: 1.45;
    background: var(--success-soft);
    border: 1px solid var(--success-border);
    border-radius: var(--radius-sm);
  }

  .help-text {
    margin: 0;
    color: var(--text-subtle);
    font-size: 0.75rem;
    line-height: 1.45;
  }

  .pipeline-fields {
    margin-top: 1rem;
  }

  .pipeline-note {
    margin-top: 0.625rem;
  }

  .keyring-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    min-height: 2.75rem;
  }

  .keyring-row input {
    flex: 1;
  }

  .keyring-status {
    flex: 1;
    color: var(--success);
    font-size: 0.8125rem;
    font-weight: 500;
  }

  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0.375rem;
    min-height: 2.75rem;
    padding: 0.625rem 0.875rem;
    color: var(--text-strong);
    font: inherit;
    font-size: 0.8125rem;
    font-weight: 650;
    line-height: 1.25;
    white-space: nowrap;
    cursor: pointer;
    background: var(--primary);
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    transition: background-color 160ms ease-out, border-color 160ms ease-out, box-shadow 160ms ease-out;
  }

  .btn:hover {
    background: var(--primary-hover);
  }
  .btn:disabled {
    cursor: not-allowed;
    opacity: 0.55;
  }


  .btn:focus-visible {
    outline: none;
    box-shadow: 0 0 0 3px var(--focus-ring-strong);
  }

  .btn-small {
    min-height: 2.5rem;
    padding: 0.5rem 0.75rem;
    font-size: 0.75rem;
  }

  .btn-secondary {
    color: var(--text);
    background: var(--surface-raised);
    border-color: var(--border);
  }

  .btn-secondary:hover {
    background: var(--border);
    border-color: var(--border-strong);
  }

  .btn-danger {
    color: var(--text-strong);
    background: var(--danger);
  }

  .btn-danger:hover {
    background: var(--danger-hover);
  }

  .storage-metrics {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 0.75rem;
    margin: 0 0 1rem;
  }

  .metric-card {
    min-width: 0;
    padding: 0.875rem;
    background: var(--page-bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }

  .metric-card dt {
    overflow: hidden;
    color: var(--text-subtle);
    font-size: 0.6875rem;
    font-weight: 650;
    letter-spacing: 0.04em;
    text-overflow: ellipsis;
    text-transform: uppercase;
    white-space: nowrap;
  }

  .metric-card dd {
    margin: 0.375rem 0 0;
    overflow: hidden;
    color: var(--text-strong);
    font-size: 1rem;
    font-weight: 650;
    letter-spacing: -0.015em;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .maintenance-panel {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    padding: 1rem;
    background: var(--surface-soft);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }

  .maintenance-copy h4,
  .danger-zone h4 {
    margin-bottom: 0.25rem;
    color: var(--text);
    font-size: 0.8125rem;
    font-weight: 650;
  }

  .maintenance-copy p,
  .danger-zone p {
    margin-bottom: 0;
    color: var(--text-subtle);
    font-size: 0.75rem;
    line-height: 1.45;
  }

  .storage-actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 0.5rem;
  }

  .danger-zone {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    margin-top: 1rem;
    padding: 1rem;
    background: var(--danger-soft);
    border: 1px solid var(--danger-border);
    border-radius: var(--radius-md);
  }

  .danger-zone h4 {
    color: var(--danger-text);
  }

  .danger-zone p {
    color: var(--danger-text-muted);
  }

  .success-text {
    margin: 0.5rem 0 0;
    color: var(--success);
    font-size: 0.75rem;
    line-height: 1.45;
  }

  .storage-status {
    padding: 0.625rem 0.75rem;
    background: var(--success-soft);
    border: 1px solid var(--success-border);
    border-radius: var(--radius-sm);
  }

  .loading {
    padding: 1rem;
    border-radius: var(--radius-md);
    font-size: 0.8125rem;
    color: var(--text-muted);
    text-align: center;
    background: var(--surface-subtle);
    border: 1px solid var(--border);
  }


  .reset-modal-overlay {
    position: fixed;
    inset: 0;
    z-index: 90;
    display: grid;
    place-items: center;
    padding: 1rem;
    background: var(--overlay);
  }

  .reset-modal-backdrop {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    padding: 0;
    cursor: default;
    background: transparent;
    border: 0;
  }

  .reset-modal-backdrop:focus-visible {
    outline: 2px solid var(--primary-ring);
    outline-offset: -0.5rem;
  }

  .reset-modal {
    position: relative;
    z-index: 1;
    width: min(100%, 28rem);
    padding: 1.5rem;
    background: var(--surface);
    border: 1px solid var(--danger-border-strong);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-modal);
  }

  .reset-modal h4 {
    margin-bottom: 0.5rem;
    color: var(--text-strong);
    font-size: 1.25rem;
    font-weight: 650;
    letter-spacing: -0.02em;
  }

  .reset-modal > p:not(.modal-eyebrow) {
    margin-bottom: 1.25rem;
    color: var(--text-muted);
    font-size: 0.875rem;
    line-height: 1.55;
  }

  .checkbox-label {
    align-items: flex-start;
    gap: 0.625rem;
    padding: 0.75rem;
    color: var(--text);
    background: var(--page-bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    cursor: pointer;
  }

  .checkbox-label input {
    width: 1rem;
    height: 1rem;
    margin: 0.0625rem 0 0;
    accent-color: var(--primary);
  }

  .checkbox-label span {
    color: var(--text);
    font-weight: 600;
  }

  .reset-modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
    margin-top: 1.25rem;
  }

  @media (max-width: 48rem) {
    .settings-container {
      padding: 2rem 1rem;
    }

    .storage-metrics {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }

    .maintenance-panel {
      align-items: flex-start;
      flex-direction: column;
    }

    .storage-actions {
      justify-content: flex-start;
    }
  }

  @media (max-width: 42rem) {
    .inference-picker {
      grid-template-columns: 1fr;
    }

    .inference-heading-content {
      align-items: flex-start;
      flex-direction: column;
    }

    .inference-status {
      width: 100%;
      justify-content: space-between;
    }
  }

  @media (max-width: 34rem) {
    .pipeline-fields {
      grid-template-columns: 1fr;
    }
    .provider-endpoint-fields {
      grid-template-columns: 1fr;
    }



    .restore-defaults {
      align-items: flex-start;
      flex-direction: column;
    }

    .keyring-row,
    .danger-zone {
      align-items: stretch;
      flex-direction: column;
    }

    .keyring-status {
      min-height: 2.5rem;
      display: flex;
      align-items: center;
    }

    .storage-actions,
    .storage-actions .btn,
    .danger-zone .btn {
      width: 100%;
    }

    .reset-modal-actions {
      flex-direction: column-reverse;
    }

    .reset-modal-actions .btn {
      width: 100%;
    }
  }
</style>
