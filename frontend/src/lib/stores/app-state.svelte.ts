import { listen } from '@tauri-apps/api/event';
import { api } from '$lib/api';

type AppPhase = 'starting' | 'downloading_model' | 'ready' | 'error';

interface DownloadProgress {
  model_id: string;
  current_file: string;
  file_downloaded: number;
  file_total: number;
  files_completed: number;
  total_files: number;
  type: string;
  message?: string;
}

function formatBytes(bytes: number) {
  const megabyte = 1024 * 1024;
  const gigabyte = megabyte * 1024;

  if (bytes >= gigabyte) return `${(bytes / gigabyte).toFixed(1)} GB`;
  return `${(bytes / megabyte).toFixed(0)} MB`;
}

export class AppState {
  phase = $state<AppPhase>('starting');
  downloadMessage = $state('');
  downloadProgress = $state(0);
  downloadIsIndeterminate = $state(true);
  errorMessage = $state('');
  embedderAvailable = $state(false);
  llmReady = $state(false);
  modelError = $state('');

  private unlisten: Array<() => void> = [];

  async init() {
    try {
      const unlistenEmbedder = await listen('embedder_ready', () => {
        console.log('[AppState] embedder ready');
        this.embedderAvailable = true;
        this.#tryBecomeReady();
      });

      const unlistenEmbedderFailed = await listen<{ message: string }>('embedder_failed', (event) => {
        console.error('[AppState] embedder failed:', event.payload.message);
        this.phase = 'error';
        this.errorMessage = event.payload.message;
      });

      const unlistenProgress = await listen<DownloadProgress>('model_download_progress', (event) => {
        const p = event.payload;
        console.log('[AppState] download progress:', p);

        if (p.type === 'error') {
          this.modelError = p.message || `Model download failed: ${p.model_id}`;
          this.downloadIsIndeterminate = false;
          if (!this.embedderAvailable) {
            this.phase = 'error';
            this.errorMessage = this.modelError;
          }
          return;
        }

        if (p.type === 'fetching_info') {
          if (this.phase !== 'error' && !this.ingestionAvailable) this.phase = 'downloading_model';
          this.downloadProgress = 0;
          this.downloadIsIndeterminate = true;
          this.downloadMessage = `Fetching model info for ${p.model_id}...`;
          return;
        }

        if (p.type === 'downloading') {
          if (this.phase !== 'error' && !this.ingestionAvailable) this.phase = 'downloading_model';
          if (p.file_total > 0) {
            this.downloadProgress = Math.min(100, Math.round((p.file_downloaded / p.file_total) * 100));
            this.downloadIsIndeterminate = false;
          } else {
            this.downloadIsIndeterminate = true;
          }
          const transferred = formatBytes(p.file_downloaded);
          const total = p.file_total > 0 ? ` of ${formatBytes(p.file_total)}` : '';
          const percentage = p.file_total > 0 ? ` (${this.downloadProgress}%)` : '';
          this.downloadMessage = `Downloading ${p.current_file} (${transferred}${total}${percentage})`;
          return;
        }

        if (p.type === 'complete') {
          return;
        }

        if (p.type === 'loading_model') {
          if (this.phase !== 'error' && !this.ingestionAvailable) this.phase = 'downloading_model';
          this.downloadProgress = 100;
          this.downloadIsIndeterminate = true;
          this.downloadMessage = 'Loading the local model...';
          return;
        }
      });

      const unlistenLlm = await listen('llm_ready', () => {
        console.log('[AppState] LLM ready');
        this.llmReady = true;
        this.#tryBecomeReady();
      });
      const unlistenLlmUnready = await listen('llm_unready', () => {
        console.log('[AppState] local LLM initialization requested');
        this.llmReady = false;
        this.modelError = '';
      });

      this.unlisten = [unlistenEmbedder, unlistenEmbedderFailed, unlistenProgress, unlistenLlm, unlistenLlmUnready];

      // If both were already ready (e.g. BYO mode with cached embedder), check now
      this.#tryBecomeReady();

      // All listeners are set up — kick off first-run init (embedder + LLM download)
      console.log('[AppState] Listeners ready, triggering first-run init');
      api.beginFirstRunInit().catch((e) => {
        console.error('[AppState] begin_first_run_init failed:', e);
        this.phase = 'error';
        this.errorMessage = `Failed to start initialization: ${e}`;
      });
    } catch (e) {
      console.error('[AppState] Failed to init event listeners:', e);
      this.phase = 'error';
      this.errorMessage = `Failed to initialize: ${e}`;
    }
  }

  get ingestionAvailable() {
    return this.embedderAvailable;
  }

  get queryAvailable() {
    return this.llmReady;
  }

  #tryBecomeReady() {
    if (this.phase !== 'error' && this.ingestionAvailable) {
      this.phase = 'ready';
    }
  }

  destroy() {
    this.unlisten.forEach(fn => fn());
    this.unlisten = [];
  }
}

export const appState = new AppState();
