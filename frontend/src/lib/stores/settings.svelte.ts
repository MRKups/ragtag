import { api } from '../api';

export class SettingsState {
  inference_mode = $state('local');
  external_base_url = $state('');
  generation_model = $state('');
  isq_level = $state('Q4K');
  use_query_enhancement = $state(true);
  chunk_size = $state(512);
  chunk_overlap = $state(80);

  isLoading = $state(true);

  async load() {
    this.isLoading = true;
    try {
      const settings = await api.getSettings();
      if (settings.inference_mode) this.inference_mode = settings.inference_mode;
      if (settings.external_base_url) this.external_base_url = settings.external_base_url;
      if (settings.generation_model) this.generation_model = settings.generation_model;
      if (settings.isq_level) this.isq_level = settings.isq_level;
      if (settings.use_query_enhancement) this.use_query_enhancement = settings.use_query_enhancement === 'true';
      if (settings.chunk_size) this.chunk_size = parseInt(settings.chunk_size, 10);
      if (settings.chunk_overlap) this.chunk_overlap = parseInt(settings.chunk_overlap, 10);
    } catch (e) {
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
      if (key === 'generation_model') this.generation_model = value;
      if (key === 'isq_level') this.isq_level = value;
      if (key === 'chunk_size') this.chunk_size = parseInt(value, 10);
      if (key === 'chunk_overlap') this.chunk_overlap = parseInt(value, 10);
    } catch (e) {
      console.error(`Failed to update setting ${key}`, e);
    }
  }
}

export const settingsState = new SettingsState();
