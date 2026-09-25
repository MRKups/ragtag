<script lang="ts">
  import QueryInput from '$lib/components/QueryInput.svelte';
  import QueryResult from '$lib/components/QueryResult.svelte';
  import DocumentViewer from '$lib/components/DocumentViewer.svelte';
  import { api } from '$lib/api';
  import { queryState } from '$lib/stores/query.svelte';
  import { appState } from '$lib/stores/app-state.svelte';
  import type { AppErrorPayload, TokenEvent } from '$lib/types';
  import { errorDialog } from '$lib/stores/error-dialog.svelte';
  import { errorMessage, isAppError, isTransientError } from '$lib/error';
  import { router } from '$lib/router.svelte';
  import { settingsState } from '$lib/stores/settings.svelte';

  function openRemoteSettings() {
    settingsState.openInferenceConfiguration('external');
    router.navigate('settings');
  }


  function reportQueryFailure(streamStarted: boolean, noEvidence = false, customMessage?: string, appErr?: AppErrorPayload | null) {
    if (!noEvidence && settingsState.inference_mode === 'external') {
      errorDialog.report({
        dedupeKey: 'external-inference-readiness',
        title: appErr?.operation ?? (streamStarted ? 'Remote answer interrupted' : 'Remote provider unavailable'),
        message: customMessage || (streamStarted
          ? 'The configured remote provider stopped responding before the answer finished.'
          : 'Ragtag could not start an answer with the configured remote provider.'),
        guidance: appErr?.recovery ?? 'Check that the provider is running and that its endpoint, model name, and API key are correct.',
        code: appErr?.code ?? 'inference',
        transient: appErr ? isTransientError(appErr) : true,
        action: { label: 'Open remote settings', run: openRemoteSettings }
      });
      return;
    }

    errorDialog.report({
      dedupeKey: streamStarted ? 'query-stream-failed' : 'query-request-failed',
      title: appErr?.operation ?? (streamStarted ? 'Answer interrupted' : 'Ragtag could not answer that question'),
      message: customMessage || (streamStarted
        ? 'The answer stopped before Ragtag finished generating it.'
        : 'Ragtag could not prepare an answer from your indexed documents.'),
      guidance: appErr?.recovery ?? (streamStarted
        ? 'Any completed text and sources remain available. Try the question again if you need a complete answer.'
        : 'Make sure at least one relevant document has finished indexing, then try again.'),
      code: appErr?.code,
      transient: appErr ? isTransientError(appErr) : false,
      action: {
        label: 'Open documents',
        run: () => router.navigate('documents')
      }
    });
  }

  let chatScrollArea: HTMLDivElement | null = $state(null);
  let userScrolledUp = false;

  function handleScroll() {
    if (!chatScrollArea) return;
    const distanceToBottom = chatScrollArea.scrollHeight - chatScrollArea.scrollTop - chatScrollArea.clientHeight;
    userScrolledUp = distanceToBottom > 80;
  }

  $effect(() => {
    if (chatScrollArea && (queryState.isStreaming || queryState.isLoading)) {
      const _ = queryState.text;
      if (!userScrolledUp) {
        chatScrollArea.scrollTop = chatScrollArea.scrollHeight;
      }
    }
  });

  async function handleQuery(text: string) {
    userScrolledUp = false;
    if (queryState.currentRequestId) {
      void api.cancelQuery(queryState.currentRequestId);
    }
    queryState.reset();
    const requestId = crypto.randomUUID();
    queryState.currentRequestId = requestId;
    queryState.queryText = text;
    queryState.isLoading = true;
    queryState.isStreaming = true;

    try {
      await api.executeQuery(text, requestId, (event: TokenEvent) => {
        if (queryState.currentRequestId !== requestId) return;
        if (event.type === 'token') {
          queryState.text += event.text;
        } else if (event.type === 'warning') {
          queryState.retrievalWarning = event.message;
        } else if (event.type === 'done') {
          queryState.complete(event.citations);
        } else if (event.type === 'error') {
          reportQueryFailure(true, false, event.message);
          queryState.citations = event.citations;
          queryState.currentRequestId = null;
          queryState.isStreaming = false;
          queryState.isLoading = false;
        }
      });
    } catch (error) {
      if (queryState.currentRequestId !== requestId) return;
      const appErr = isAppError(error) ? error : null;
      const message = appErr?.detail ?? errorMessage(error);
      const noEvidence = appErr
        ? appErr.code === 'inference' && appErr.detail.includes('No usable document excerpts')
        : message.includes('No usable document excerpts were retrieved');
      reportQueryFailure(false, noEvidence, message, appErr);
      queryState.currentRequestId = null;
      queryState.isStreaming = false;
      queryState.isLoading = false;
    }
  }

  function handleCancel() {
    if (queryState.currentRequestId) {
      void api.cancelQuery(queryState.currentRequestId);
    }
    queryState.cancel();
  }

  $effect(() => {
    return () => {
      if (queryState.currentRequestId) {
        void api.cancelQuery(queryState.currentRequestId);
        queryState.stopGenerating();
      }
    };
  });

  function handleCitationClick(detail: { filename: string; storagePath: string; documentId: string; page: number }) {
    queryState.selectedDocument = detail;
  }

</script>

<div class="page-container">
  <div class="panel left-panel">
    <div class="chat-scroll-area" bind:this={chatScrollArea} onscroll={handleScroll}>
      {#if queryState.isLoading && !queryState.text}
        <div class="loading-state">
          <div class="spinner"></div>
          <p>Searching manuals...</p>
        </div>
      {:else if queryState.text || queryState.citations.length > 0 || queryState.retrievalWarning}
        {#if queryState.retrievalWarning}
          <p class="retrieval-warning" role="status">{queryState.retrievalWarning}</p>
        {/if}
        <QueryResult
          text={queryState.text}
          citations={queryState.citations}
          isStreaming={queryState.isStreaming}
          onCitationClick={handleCitationClick}
        />
      {:else}
        <div class="welcome-state">
          <h2>How can I help you today?</h2>
          <p>Ask questions about your technical manuals.</p>
        </div>
      {/if}
    </div>

    <div class="chat-input-area">
      <div class="input-wrapper-styled">
        {#if appState.isDownloadingModel}
          <div class="downloading-banner">
            <span class="banner-text">{appState.downloadMessage || 'Downloading model in the background...'}</span>
            <button
              type="button"
              class="cancel-banner-btn"
              onclick={() => void appState.cancelModelDownload()}
            >
              Cancel Download
            </button>
          </div>
        {/if}
        <QueryInput
          isLoading={queryState.isLoading}
          isStreaming={queryState.isStreaming}
          unavailable={!appState.queryAvailable}
          unavailableMessage={appState.isDownloadingModel
            ? `Downloading model: ${appState.downloadMessage || 'Please wait...'}`
            : 'The selected inference model is unavailable. Resolve the issue in the dialog to continue.'}
          onSubmit={handleQuery}
          onCancel={handleCancel}
          bind:text={queryState.queryText}
        />
      </div>
    </div>
  </div>

  <div class="panel right-panel">
    <DocumentViewer
      filename={queryState.selectedDocument?.filename ?? null}
      storagePath={queryState.selectedDocument?.storagePath ?? null}
      page={queryState.selectedDocument?.page ?? 1}
      documentId={queryState.selectedDocument?.documentId ?? null}
    />
  </div>
</div>

<style>
  .page-container {
    display: flex;
    width: 100%;
    height: 100%;
  }

  .panel {
    width: 50%;
    height: 100%;
    display: flex;
    flex-direction: column;
  }

  .left-panel {
    border-right: 1px solid #374151;
  }

  .right-panel {
    background-color: #0f172a;
  }

  .chat-scroll-area {
    flex: 1;
    overflow-y: auto;
    padding: 2rem;
  }

  .chat-input-area {
    padding: 1.5rem 2rem;
    border-top: 1px solid #1e293b;
  }

  .input-wrapper-styled {
    background-color: #1e293b;
    border-radius: 1rem;
    border: 1px solid #334155;
    padding: 1rem;
    box-shadow: 0 4px 6px -1px rgba(0, 0, 0, 0.1);
    max-width: 800px;
    margin: 0 auto;
    width: 100%;
  }

  .loading-state, .welcome-state {

    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    text-align: center;
  }

  .welcome-state h2 {
    color: #e5e7eb;
    font-size: 1.5rem;
    font-weight: 600;
    margin-bottom: 0.5rem;
  }

  .welcome-state p {
    color: #9ca3af;
    font-size: 1.05rem;
  }

  .retrieval-warning {
    margin: 0 0 1rem;
    border-left: 3px solid #f59e0b;
    padding: 0.75rem 1rem;
    color: #fde68a;
    background: #422006;
    border-radius: 0.25rem;
  }

  .spinner {
    width: 3rem;
    height: 3rem;
    border: 4px solid #1e3a8a;
    border-top-color: #3b82f6;
    border-radius: 50%;
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .downloading-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    padding: 0.5rem 0.75rem;
    margin-bottom: 0.75rem;
    background-color: #1e293b;
    border: 1px solid #3b82f6;
    border-radius: 0.5rem;
  }

  .banner-text {
    font-size: 0.8125rem;
    color: #93c5fd;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .cancel-banner-btn {
    flex-shrink: 0;
    padding: 0.25rem 0.625rem;
    background-color: transparent;
    color: #9ca3af;
    border: 1px solid #4b5563;
    border-radius: 0.25rem;
    font-size: 0.75rem;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.2s;
  }

  .cancel-banner-btn:hover {
    color: #f3f4f6;
    border-color: #9ca3af;
    background-color: #374151;
  }
</style>
