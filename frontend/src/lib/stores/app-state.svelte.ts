import { listen } from '@tauri-apps/api/event';

type AppPhase = 'starting' | 'downloading_model' | 'ready' | 'error';

interface DownloadProgress {
  model_id: string;
  current_file: string;
  file_downloaded: number;
  file_total: number;
  files_completed: number;
  total_files: number;
  stage: string;
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

      const unlistenProgress = await listen<DownloadProgress>('model_download_progress', (event) => {
        const p = event.payload;
        console.log('[AppState] download progress:', p);

        if (p.stage === 'error') {
          this.phase = 'error';
          this.errorMessage = `Model download failed: ${p.model_id}`;
          return;
        }

        if (p.stage === 'fetching_info') {
          this.phase = 'downloading_model';
          this.downloadMessage = `Fetching model info for ${p.model_id}...`;
          return;
        }

        if (p.stage === 'downloading') {
          this.phase = 'downloading_model';
          if (p.file_total > 0) {
            this.downloadProgress = Math.round((p.file_downloaded / p.file_total) * 100);
          }
          const mb = (p.file_downloaded / (1024 * 1024)).toFixed(0);
          this.downloadMessage = `Downloading ${p.current_file} (${mb} MB)`;
          return;
        }

        if (p.stage === 'complete') {
          this.downloadMessage = 'Download complete, loading model...';
          return;
        }
      });

      const unlistenLlm = await listen('llm_ready', () => {
        console.log('[AppState] LLM ready');
        this.phase = 'ready';
      });

      this.unlisten = [unlistenEmbedder, unlistenProgress, unlistenLlm];

      // If LLM was already ready before we subscribed (race), check state
      // The embedder_ready fires synchronously, so we're in 'downloading_model' or 'ready' already
      // If no download_progress arrives within a short window, we might be in BYO mode
      setTimeout(() => {
        if (this.phase === 'starting') {
          // embedder_ready fired but no download progress — likely BYO mode
          console.log('[AppState] No model download detected after 3s, assuming BYO or local LLM ready');
          this.phase = 'ready';
        }
      }, 3000);
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
