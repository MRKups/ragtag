<script lang="ts">
  import { onDestroy } from 'svelte';
  import QueryInput from '$lib/components/QueryInput.svelte';
  import QueryResult from '$lib/components/QueryResult.svelte';
  import DocumentViewer from '$lib/components/DocumentViewer.svelte';
  import { api } from '$lib/api';
  import { queryState } from '$lib/stores/query.svelte';
  import { appState } from '$lib/stores/app-state.svelte';
  import type { TokenEvent } from '$lib/types';
  async function handleQuery(text: string) {
    if (queryState.currentRequestId) {
      void api.cancelQuery(queryState.currentRequestId);
    }
    queryState.cancel();
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
        } else if (event.type === 'done') {
          queryState.citations = event.citations;
          queryState.isStreaming = false;
          queryState.isLoading = false;
        } else if (event.type === 'error') {
          queryState.error = { message: event.message };
          queryState.citations = event.citations;
          queryState.isStreaming = false;
          queryState.isLoading = false;
        }
      });
    } catch (e) {
      if (queryState.currentRequestId !== requestId) return;
      const msg = e && typeof e === 'object' && 'message' in e ? String(e.message) : 'Query failed';
      queryState.error = { message: msg };
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

  function handleCitationClick(detail: { filename: string; storagePath: string; documentId: string; page: number }) {
    queryState.selectedDocument = detail;
  }

  onDestroy(handleCancel);
</script>

<div class="page-container">
  <div class="panel left-panel">
    <div class="chat-scroll-area">
      {#if queryState.isLoading && !queryState.text}
        <div class="loading-state">
          <div class="spinner"></div>
          <p>Searching manuals...</p>
        </div>
      {:else if queryState.text || queryState.error}
        <QueryResult
          text={queryState.text}
          citations={queryState.citations}
          isStreaming={queryState.isStreaming}
          error={queryState.error?.message ?? null}
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
        <QueryInput
          isLoading={queryState.isLoading}
          isStreaming={queryState.isStreaming}
          unavailable={!appState.queryAvailable}
          unavailableMessage={appState.modelError || 'The selected inference model is still preparing.'}
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
</style>
