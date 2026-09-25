// Manages global application lifecycle state, model download progress, and external inference readiness.
import { api } from '$lib/api';
import { errorDialog } from '$lib/stores/error-dialog.svelte';
import { router } from '$lib/router.svelte';
import { settingsState } from '$lib/stores/settings.svelte';
import type { ModelDownloadProgress } from '$lib/types';

type AppPhase = 'starting' | 'downloading_model' | 'ready' | 'error';

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
  isDownloadingModel = $state(false);
  #localModelRetrying = false;
  #isCancellingModelDownload = false;
  #unlistenFns: (() => void)[] = [];

  #cleanupListeners() {
    for (const unlisten of this.#unlistenFns) {
      unlisten();
    }
    this.#unlistenFns = [];
  }

  destroy() {
    this.#cleanupListeners();
  }

  #applyDownloadProgress(p: ModelDownloadProgress) {
    if (this.#isCancellingModelDownload) {
      return;
    }
    if (p.type === 'error') {
      this.isDownloadingModel = false;
      this.#localModelRetrying = false;
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
      this.isDownloadingModel = true;
      if (this.phase !== 'error' && !this.ingestionAvailable) this.phase = 'downloading_model';
      this.downloadProgress = 0;
      this.downloadIsIndeterminate = true;
      this.downloadMessage = `Fetching model info for ${p.model_id}...`;
      return;
    }

    if (p.type === 'downloading') {
      this.isDownloadingModel = true;
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
      this.isDownloadingModel = false;
      return;
    }

    if (p.type === 'loading_model') {
      this.isDownloadingModel = true;
      if (this.phase !== 'error' && !this.ingestionAvailable) this.phase = 'downloading_model';
      this.downloadProgress = 100;
      this.downloadIsIndeterminate = true;
      this.downloadMessage = 'Loading the local model...';
      return;
    }
  }

  async reconcile() {
    try {
      const state = await api.getAppState();
      this.embedderAvailable = state.embedder_ready;
      this.llmReady = state.llm_ready;
      this.isDownloadingModel = state.model_download_active;

      if (state.model_download_progress) {
        this.#applyDownloadProgress(state.model_download_progress);
      } else if (!state.model_download_active && this.phase === 'downloading_model') {
        this.downloadMessage = '';
        this.downloadProgress = 0;
      }

      this.#tryBecomeReady();
    } catch (err) {
      console.warn('[AppState] Reconcile probe failed:', err);
    }
  }

  async cancelModelDownload() {
    try {
      this.#isCancellingModelDownload = true;
      await api.cancelModelDownload();
      this.isDownloadingModel = false;
      this.downloadMessage = '';
      this.downloadProgress = 0;
      this.#localModelRetrying = false;
      this.#tryBecomeReady();
    } catch (err) {
      this.#isCancellingModelDownload = false;
      console.error('[AppState] cancelModelDownload failed:', err);
      errorDialog.reportError(err, 'Cancel model download');
    }
  }

  async init() {
    this.#cleanupListeners();
    try {
      this.#unlistenFns.push(
        await api.listen('embedder_ready', () => {
          this.embedderAvailable = true;
          this.#tryBecomeReady();
        })
      );

      this.#unlistenFns.push(
        await api.listen<{ message: string }>('embedder_failed', () => {
          console.error('[AppState] embedder failed');
          this.embedderAvailable = false;
          this.phase = 'error';
          this.errorMessage = 'Ragtag could not initialize its document tools. Check your internet connection and local storage, then try again.';
        })
      );

      this.#unlistenFns.push(
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
        })
      );

      this.#unlistenFns.push(
        await api.listen<ModelDownloadProgress>('model_download_progress', (event) => {
          this.#applyDownloadProgress(event.payload);
        })
      );

      this.#unlistenFns.push(
        await api.listen('model_download_cancelled', () => {
          this.#isCancellingModelDownload = false;
          this.isDownloadingModel = false;
          this.downloadMessage = '';
          this.downloadProgress = 0;
          this.#localModelRetrying = false;
          this.#tryBecomeReady();
        })
      );

      this.#unlistenFns.push(
        await api.listen('llm_ready', () => {
          this.#isCancellingModelDownload = false;
          this.#localModelRetrying = false;
          this.llmReady = true;
          this.isDownloadingModel = false;
          errorDialog.clear('local-model-initialization');
          errorDialog.clear('external-inference-readiness');
          this.#tryBecomeReady();
        })
      );

      this.#unlistenFns.push(
        await api.listen('llm_unready', () => {
          this.#localModelRetrying = false;
          this.llmReady = false;
          errorDialog.clear('local-model-initialization');
          errorDialog.clear('external-inference-readiness');
        })
      );

      this.#unlistenFns.push(
        await api.listenExternalInferenceFailed(() => {
          console.warn('[AppState] external inference unavailable');
          this.llmReady = false;
          this.isDownloadingModel = false;
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
        })
      );

      // Synchronously reconcile initial state from lifecycle probe
      await this.reconcile();

      // Trigger first-run init if needed
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
    if (this.#localModelRetrying || this.llmReady) return;

    this.#localModelRetrying = true;
    this.downloadMessage = 'Retrying local model initialization...';
    this.downloadIsIndeterminate = true;
    try {
      await api.retryLocalModelInitialization();
    } catch {
      this.#localModelRetrying = false;
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
