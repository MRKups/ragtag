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

export class AppState {
  phase = $state<AppPhase>('starting');
  downloadMessage = $state('');
  downloadProgress = $state(0);
  errorMessage = $state('');

  private unlisten: Array<() => void> = [];

  async init() {
    try {
      const unlistenEmbedder = await listen('embedder_ready', () => {
        console.log('[AppState] embedder ready');
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
          this.phase = 'error';
          this.errorMessage = p.message || `Model download failed: ${p.model_id}`;
          return;
        }

        if (p.type === 'fetching_info') {
          this.phase = 'downloading_model';
          this.downloadMessage = `Fetching model info for ${p.model_id}...`;
          return;
        }

        if (p.type === 'downloading') {
          this.phase = 'downloading_model';
          if (p.file_total > 0) {
            this.downloadProgress = Math.round((p.file_downloaded / p.file_total) * 100);
          }
          const mb = (p.file_downloaded / (1024 * 1024)).toFixed(0);
          this.downloadMessage = `Downloading ${p.current_file} (${mb} MB)`;
          return;
        }

        if (p.type === 'complete') {
          this.phase = 'downloading_model';
          this.downloadMessage = 'Download complete, loading model...';
          return;
        }
      });

      const unlistenLlm = await listen('llm_ready', () => {
        console.log('[AppState] LLM ready');
        if (this.phase !== 'error') {
          this.phase = 'ready';
        }
      });

      this.unlisten = [unlistenEmbedder, unlistenEmbedderFailed, unlistenProgress, unlistenLlm];

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

  destroy() {
    this.unlisten.forEach(fn => fn());
    this.unlisten = [];
  }
}

export const appState = new AppState();
