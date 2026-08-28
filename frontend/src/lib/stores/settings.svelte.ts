import { api } from '../api';

export class SettingsState {
  inference_mode = $state('local');
  external_base_url = $state('');
  external_model = $state('gpt-4o-mini');
  generation_model = $state('');
  tokenizer_model = $state('Qwen/Qwen3-1.7B');
  gguf_filename = $state('Qwen3-1.7B-Q4_K_M.gguf');
  use_query_enhancement = $state(true);
  chunk_size = $state(512);
  chunk_overlap = $state(80);

  isLoading = $state(true);
  loadError = $state<string | null>(null);
  saveError = $state<string | null>(null);

  async load() {
    this.isLoading = true;
    try {
      const settings = await api.getSettings();
      if (settings.inference_mode) this.inference_mode = settings.inference_mode;
      if (settings.external_base_url) this.external_base_url = settings.external_base_url;
      if (settings.external_model) this.external_model = settings.external_model;
      if (settings.generation_model) this.generation_model = settings.generation_model;
      if (settings.tokenizer_model) this.tokenizer_model = settings.tokenizer_model;
      if (settings.gguf_filename) this.gguf_filename = settings.gguf_filename;
      if (settings.use_query_enhancement) this.use_query_enhancement = settings.use_query_enhancement === 'true';
      if (settings.chunk_size) this.chunk_size = parseInt(settings.chunk_size, 10);
      if (settings.chunk_overlap) this.chunk_overlap = parseInt(settings.chunk_overlap, 10);
    } catch (e) {
      const msg = e && typeof e === 'object' && 'message' in e ? String(e.message) : 'Unknown error';
      this.loadError = `Failed to load settings: ${msg}`;
      console.error("Failed to load settings", e);
    } finally {
      this.isLoading = false;
    }
  }

  async update(key: string, value: string) {
    try {
      await api.setSetting(key, value);
      // Update local state based on key
      if (key === 'use_query_enhancement') this.use_query_enhancement = value === 'true';
      if (key === 'inference_mode') this.inference_mode = value;
      if (key === 'external_base_url') this.external_base_url = value;
      if (key === 'external_model') this.external_model = value;
      if (key === 'generation_model') this.generation_model = value;
      if (key === 'tokenizer_model') this.tokenizer_model = value;
      if (key === 'gguf_filename') this.gguf_filename = value;
      if (key === 'chunk_size') this.chunk_size = parseInt(value, 10);
      if (key === 'chunk_overlap') this.chunk_overlap = parseInt(value, 10);
    } catch (e) {
      const msg = e && typeof e === 'object' && 'message' in e ? String(e.message) : 'Unknown error';
      this.saveError = `Failed to save ${key}: ${msg}`;
      console.error(`Failed to update setting ${key}`, e);
      setTimeout(() => { this.saveError = null; }, 5000);
    }
  }
}

export const settingsState = new SettingsState();
