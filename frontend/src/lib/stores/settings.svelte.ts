// Synchronizes inference settings and chunk configuration with persistent backend storage.
import { api } from '../api.ts';
import { errorDialog } from './error-dialog.svelte.ts';
import { isAppError } from '../error.ts';

export const DEFAULT_LOCAL_INFERENCE = {
  generation_model: 'unsloth/Qwen3-1.7B-GGUF',
  tokenizer_model: 'Qwen/Qwen3-1.7B',
  gguf_filename: 'Qwen3-1.7B-Q4_K_M.gguf'
} as const;

export type InferenceConfiguration = 'local' | 'external';

function isQueueFull(error: unknown): boolean {
  if (isAppError(error) && error.code === 'queue_full') return true;
  if (
    typeof error === 'object'
    && error !== null
    && 'kind' in error
    && (error as { kind: string }).kind === 'QueueFull'
  ) {
    return true;
  }
  return false;
}

export class SettingsState {
  inference_mode = $state('local');
  external_base_url = $state('');
  external_model = $state('gpt-4o-mini');
  generation_model = $state<string>(DEFAULT_LOCAL_INFERENCE.generation_model);
  tokenizer_model = $state<string>(DEFAULT_LOCAL_INFERENCE.tokenizer_model);
  gguf_filename = $state<string>(DEFAULT_LOCAL_INFERENCE.gguf_filename);
  use_query_enhancement = $state(true);
  chunk_size = $state(512);
  chunk_overlap = $state(80);

  isLoading = $state(true);
  requestedInferenceConfiguration = $state<InferenceConfiguration | null>(null);
  private pendingInferenceModeUpdate: Promise<boolean> | null = null;


  private persistedSettings: Record<string, string> = {
    inference_mode: this.inference_mode,
    external_base_url: this.external_base_url,
    external_model: this.external_model,
    generation_model: this.generation_model,
    tokenizer_model: this.tokenizer_model,
    gguf_filename: this.gguf_filename,
    use_query_enhancement: String(this.use_query_enhancement),
    chunk_size: String(this.chunk_size),
    chunk_overlap: String(this.chunk_overlap)
  };

  private applySettingValue(key: string, value: string) {
    if (key === 'use_query_enhancement') this.use_query_enhancement = value === 'true';
    if (key === 'inference_mode') this.inference_mode = value;
    if (key === 'external_base_url') this.external_base_url = value;
    if (key === 'external_model') this.external_model = value;
    if (key === 'generation_model') this.generation_model = value;
    if (key === 'tokenizer_model') this.tokenizer_model = value;
    if (key === 'gguf_filename') this.gguf_filename = value;
    if (key === 'chunk_size') this.chunk_size = parseInt(value, 10);
    if (key === 'chunk_overlap') this.chunk_overlap = parseInt(value, 10);
  }

  private currentSettingValue(key: string): string | undefined {
    if (key === 'use_query_enhancement') return String(this.use_query_enhancement);
    if (key === 'inference_mode') return this.inference_mode;
    if (key === 'external_base_url') return this.external_base_url;
    if (key === 'external_model') return this.external_model;
    if (key === 'generation_model') return this.generation_model;
    if (key === 'tokenizer_model') return this.tokenizer_model;
    if (key === 'gguf_filename') return this.gguf_filename;
    if (key === 'chunk_size') return String(this.chunk_size);
    if (key === 'chunk_overlap') return String(this.chunk_overlap);
  }

  private persistSettingValue(key: string, value: string) {
    this.persistedSettings[key] = value;
    this.applySettingValue(key, value);
  }

  restorePersistedValue(key: string) {
    const persistedValue = this.persistedSettings[key];
    if (persistedValue === undefined) return;

    this.applySettingValue(key, persistedValue);
  }

  setExternalProvider(baseUrl: string, model: string) {
    this.persistSettingValue('external_base_url', baseUrl);
    this.persistSettingValue('external_model', model);
  }


  async load() {
    this.isLoading = true;
    try {
      const settings = await api.getSettings();
      if (settings.inference_mode != null) this.persistSettingValue('inference_mode', settings.inference_mode);
      if (settings.external_base_url != null) this.persistSettingValue('external_base_url', settings.external_base_url);
      if (settings.external_model != null) this.persistSettingValue('external_model', settings.external_model);
      if (settings.generation_model != null) this.persistSettingValue('generation_model', settings.generation_model);
      if (settings.tokenizer_model != null) this.persistSettingValue('tokenizer_model', settings.tokenizer_model);
      if (settings.gguf_filename != null) this.persistSettingValue('gguf_filename', settings.gguf_filename);
      if (settings.use_query_enhancement != null) this.persistSettingValue('use_query_enhancement', settings.use_query_enhancement);
      if (settings.chunk_size != null) this.persistSettingValue('chunk_size', settings.chunk_size);
      if (settings.chunk_overlap != null) this.persistSettingValue('chunk_overlap', settings.chunk_overlap);
    } catch {
      errorDialog.report({
        dedupeKey: 'settings-load',
        title: 'Settings unavailable',
        message: 'Ragtag could not load your settings.',
        guidance: 'Try again. If this continues after restarting Ragtag, check that its local data directory is available.'
      });
      console.error('Failed to load settings');
    } finally {
      this.isLoading = false;
    }
  }

  async update(key: string, value: string): Promise<boolean> {
    if ((key === 'chunk_size' || key === 'chunk_overlap') && !value.trim()) {
      this.restorePersistedValue(key);
      return false;
    }

    try {
      await api.setSetting(key, value);
      this.persistSettingValue(key, value);
      return true;
    } catch (error) {
      if ((key === 'chunk_size' || key === 'chunk_overlap') && isQueueFull(error)) {
        this.persistedSettings[key] = value;
        if (this.currentSettingValue(key)?.trim() === value) {
          this.applySettingValue(key, value);
        }
        return true;
      }

      errorDialog.report({
        dedupeKey: `settings-save-${key}`,
        title: 'Setting not saved',
        message: 'Ragtag could not save this setting. Your previous setting is still in effect.',
        guidance: 'Check the value and try again.'
      });
      if (this.currentSettingValue(key)?.trim() === value) {
        this.restorePersistedValue(key);
      }
      return false;
    }
  }

  updateInferenceMode(mode: InferenceConfiguration): Promise<boolean> {
    const previous = this.pendingInferenceModeUpdate;
    if (!previous && mode === this.inference_mode) return Promise.resolve(true);

    const pending = (async () => {
      if (previous) await previous;
      return this.update('inference_mode', mode);
    })();

    this.pendingInferenceModeUpdate = pending;
    void pending.finally(() => {
      if (this.pendingInferenceModeUpdate === pending) {
        this.pendingInferenceModeUpdate = null;
      }
    });

    return pending;
  }

  openInferenceConfiguration(mode: InferenceConfiguration) {
    this.requestedInferenceConfiguration = mode;
  }

  consumeInferenceConfiguration(): InferenceConfiguration | null {
    const requested = this.requestedInferenceConfiguration;
    this.requestedInferenceConfiguration = null;
    return requested;
  }
}

export const settingsState = new SettingsState();
