import { api } from '$lib/api';
import { errorDialog } from '$lib/stores/error-dialog.svelte';
import { router } from '$lib/router.svelte';
import { settingsState } from '$lib/stores/settings.svelte';

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
  localModelRetrying = $state(false);


  async init() {
    try {
      await api.listen('embedder_ready', () => {
        console.log('[AppState] embedder ready');
        this.embedderAvailable = true;
        this.#tryBecomeReady();
      });

      await api.listen<{ message: string }>('embedder_failed', () => {
        console.error('[AppState] embedder failed');
        this.embedderAvailable = false;
        this.phase = 'error';
        this.errorMessage = 'Ragtag could not initialize its document tools. Check your internet connection and local storage, then try again.';
      });

      await api.listen<{ document_id: string }>('document_failed', (event) => {
        errorDialog.report({
          dedupeKey: `document-processing-${event.payload.document_id}`,
          title: 'Document processing failed',
          message: 'Ragtag could not finish indexing a document.',
          guidance: 'Review the document status, then remove and upload a new copy if needed.',
          action: {
            label: 'Open documents',
            run: () => router.navigate('documents')
          }
        });
      });

      await api.listen<DownloadProgress>('model_download_progress', (event) => {
        const p = event.payload;
        console.log('[AppState] download progress:', p);

        if (p.type === 'error') {
          this.localModelRetrying = false;
          errorDialog.report({
            dedupeKey: 'local-model-initialization',
            title: 'Local model unavailable',
            message: 'Ragtag could not start the local model.',
            guidance: 'Check your internet connection and model settings, then retry initialization.',
            action: {
              label: 'Retry local model',
              run: () => void this.retryLocalModel()
            }
          });
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

      await api.listen('llm_ready', () => {
        console.log('[AppState] LLM ready');
        this.localModelRetrying = false;
        this.llmReady = true;
        errorDialog.clear('local-model-initialization');
        errorDialog.clear('external-inference-readiness');
        this.#tryBecomeReady();
      });
      await api.listen('llm_unready', () => {
        console.log('[AppState] inference initialization requested');
        this.localModelRetrying = false;
        this.llmReady = false;
        errorDialog.clear('local-model-initialization');
        errorDialog.clear('external-inference-readiness');
      });
      await api.listenExternalInferenceFailed(() => {
        console.warn('[AppState] external inference unavailable');
        this.llmReady = false;
        errorDialog.report({
          dedupeKey: 'external-inference-readiness',
          title: 'Remote provider unavailable',
          message: 'Ragtag could not connect to the configured remote provider.',
          guidance: 'Start the provider, then verify its base URL, model name, and API key.',
          action: {
            label: 'Open remote settings',
            run: () => {
              settingsState.openInferenceConfiguration('external');
              router.navigate('settings');
            }
          }
        });
      });


      // If both were already ready (e.g. BYO mode with cached embedder), check now
      this.#tryBecomeReady();

      // All listeners are set up — kick off first-run init (embedder + LLM download)
      console.log('[AppState] Listeners ready, triggering first-run init');
      api.beginFirstRunInit().catch(() => {
        console.error('[AppState] begin_first_run_init failed');
        this.phase = 'error';
        this.errorMessage = 'Ragtag could not finish starting. Check that its local data directory is available, then try again.';
      });
    } catch {
      console.error('[AppState] Failed to init event listeners');
      this.phase = 'error';
      this.errorMessage = 'Ragtag could not initialize its desktop connection. Restart the app and try again.';
    }
  }

  async retryLocalModel() {
    if (this.localModelRetrying || this.llmReady) return;

    this.localModelRetrying = true;
    this.downloadMessage = 'Retrying local model initialization...';
    this.downloadIsIndeterminate = true;
    try {
      await api.retryLocalModelInitialization();
    } catch {
      this.localModelRetrying = false;
      errorDialog.report({
        dedupeKey: 'local-model-initialization',
        title: 'Local model retry failed',
        message: 'Ragtag could not restart the local model.',
        guidance: 'Check your model settings and try again.',
        action: {
          label: 'Retry local model',
          run: () => void this.retryLocalModel()
        }
      });
    }
  }

  get ingestionAvailable() {
    return this.embedderAvailable;
  }

  get queryAvailable() {
    return this.llmReady;
  }

  #tryBecomeReady() {
    if (this.ingestionAvailable) {
      this.phase = 'ready';
    }
  }

}

export const appState = new AppState();
