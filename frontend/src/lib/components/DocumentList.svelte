<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { FileText, RefreshCw, AlertCircle, Trash2, CheckCircle, Clock, XCircle, Loader2 } from 'lucide-svelte';
  import { api } from '$lib/api';
  import type { Document } from '$lib/types';

  let documents: Document[] = $state([]);
  let isLoading = $state(false);
  let error: string | null = $state(null);
  let deleting: string | null = $state(null);
  let confirmDeleteTarget: { id: string; filename: string } | null = $state(null);
  let pollTimeout: ReturnType<typeof setTimeout>;
  let unlistenFns: Array<() => void> = [];
  let fetchPromise: Promise<void> | null = null;

  export async function refresh() {
    await fetchDocuments();
  }

  async function fetchDocuments(silent = false) {
    if (fetchPromise) return fetchPromise;

    if (!silent) {
      isLoading = true;
      error = null;
    }

    fetchPromise = (async () => {
      try {
        const TIMEOUT_MS = 10_000;
        const listPromise = api.listDocuments();
        const timeoutPromise = new Promise<never>((_, reject) =>
          setTimeout(() => reject(new Error('Request timed out')), TIMEOUT_MS)
        );
        documents = await Promise.race([listPromise, timeoutPromise]);
        schedulePolling();
      } catch (e) {
        if (!silent) {
          error = e && typeof e === 'object' && 'message' in e ? String(e.message) : 'Failed to load documents';
        }
        clearTimeout(pollTimeout);
      } finally {
        if (!silent) isLoading = false;
        fetchPromise = null;
      }
    })();

    return fetchPromise;
  }

  function schedulePolling() {
    clearTimeout(pollTimeout);

    const hasTransientDocs = documents.some(doc =>
      ['queued', 'embedding'].includes(doc.status)
    );

    if (hasTransientDocs) {
      pollTimeout = setTimeout(() => {
        fetchDocuments(true);
      }, 3000);
    }
  }

  function confirmDelete(id: string, filename: string) {
    confirmDeleteTarget = { id, filename };
  }

  function cancelDelete() {
    confirmDeleteTarget = null;
  }

  async function executeDelete() {
    if (!confirmDeleteTarget) return;
    const { id } = confirmDeleteTarget;
    confirmDeleteTarget = null;
    deleting = id;
    try {
      await api.deleteDocument(id);
      documents = await api.listDocuments();
    } catch (e) {
      const msg = e && typeof e === 'object' && 'message' in e ? String(e.message) : 'Unknown error';
      error = `Failed to delete: ${msg}`;
    } finally {
      deleting = null;
    }
  }

  onMount(async () => {
    await fetchDocuments();

    try {
      const [unlisten1, unlisten2] = await Promise.all([
        listen('document_indexed', () => fetchDocuments(true)),
        listen('document_failed', () => fetchDocuments(true)),
      ]);
      unlistenFns = [unlisten1, unlisten2];
    } catch (err) {
      console.error('Failed to register document event listeners:', err);
    }
  });

  onDestroy(() => {
    clearTimeout(pollTimeout);
    for (const fn of unlistenFns) fn();
  });
</script>

<div class="document-list-container">
  <div class="header">
    <h3 class="title">
      <FileText size={20} />
      Document Library
    </h3>
    <button 
      type="button"
      class="refresh-btn" 
      onclick={() => fetchDocuments(false)} 
      disabled={isLoading}
      title="Refresh list"
    >
      <span class={isLoading ? 'spin' : ''}>
        <RefreshCw size={16} />
      </span>
    </button>
  </div>

  <div class="content">
    {#if confirmDeleteTarget}
      <div class="confirm-overlay" role="dialog" tabindex="-1" onclick={cancelDelete} onkeydown={(e) => e.key === 'Escape' && cancelDelete()}>
        <div class="confirm-dialog" role="presentation" onclick={(e) => e.stopPropagation()}>
          <p class="confirm-text">Delete "<strong>{confirmDeleteTarget.filename}</strong>"?</p>
          <p class="confirm-sub">This action cannot be undone.</p>
          <div class="confirm-actions">
            <button type="button" class="confirm-cancel-btn" onclick={cancelDelete}>Cancel</button>
            <button type="button" class="confirm-delete-btn" onclick={executeDelete}>
              {deleting ? 'Deleting...' : 'Delete'}
            </button>
          </div>
        </div>
      </div>
    {/if}
    {#if isLoading && documents.length === 0}
      <div class="loading-state">
        <div class="spinner"></div>
        <p>Loading documents...</p>
      </div>
    {:else if error}
      <div class="error-state">
        <AlertCircle size={20} />
        <p>{error}</p>
        <button type="button" class="retry-link" onclick={() => fetchDocuments(false)}>Retry</button>
      </div>
    {:else if documents.length === 0}
      <div class="empty-state">
        <p>No documents found.</p>
        <p class="sub-text">Upload manuals to get started.</p>
      </div>
    {:else}
      <ul class="document-list">
        {#each documents as doc (doc.id)}
          <li class="document-item">
            <span class="doc-icon-wrapper">
              <FileText size={16} />
            </span>
            <div class="doc-info">
              <div class="doc-header">
                <span class="doc-name" title={doc.filename}>{doc.filename}</span>
              </div>
              <span class="doc-status status-{doc.status.toLowerCase()}">
                {#if doc.status === 'indexed'}
                  <CheckCircle size={12} />
                {:else if doc.status === 'embedding'}
                  <Loader2 size={12} class="spin-slow" />
                {:else if doc.status === 'failed'}
                  <XCircle size={12} />
                {:else}
                  <Clock size={12} />
                {/if}
                {doc.status}
              </span>
            </div>
            <div class="actions">
              <button 
                type="button"
                class="action-btn delete-btn"
                title="Delete"
                onclick={() => confirmDelete(doc.id, doc.filename)}
                disabled={deleting === doc.id}
              >
                {#if deleting === doc.id}
                  <div class="spinner-small"></div>
                {:else}
                  <Trash2 size={16} />
                {/if}
              </button>
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</div>

<style>
  .document-list-container {
    height: 100%;
    display: flex;
    flex-direction: column;
    background-color: #1e293b;
    border-radius: 0.75rem;
    border: 1px solid #334155;
    overflow: hidden;
  }

  .header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 1rem;
    border-bottom: 1px solid #334155;
    background-color: #0f172a;
  }

  .title {
    margin: 0;
    font-size: 1rem;
    font-weight: 600;
    color: #e5e7eb;
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .refresh-btn {
    background: transparent;
    border: none;
    color: #9ca3af;
    cursor: pointer;
    padding: 0.25rem;
    border-radius: 0.25rem;
    transition: all 0.2s;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .refresh-btn:hover:not(:disabled) {
    color: white;
    background-color: #334155;
  }

  .refresh-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .content {
    flex: 1;
    overflow-y: auto;
    padding: 0;
  }

  .document-list {
    list-style: none;
    padding: 0;
    margin: 0;
  }

  .document-item {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.75rem 1rem;
    border-bottom: 1px solid #334155;
    transition: background-color 0.2s;
  }

  .document-item:last-child {
    border-bottom: none;
  }

  .document-item:hover {
    background-color: #334155;
  }

  .doc-icon-wrapper {
    color: #60a5fa;
    flex-shrink: 0;
    display: flex;
    align-items: center;
  }

  .doc-info {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    overflow: hidden;
  }

  .doc-header {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    min-width: 0;
  }

  .doc-name {
    color: #e5e7eb;
    font-size: 0.875rem;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .doc-status {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    font-size: 0.75rem;
    font-weight: 500;
    text-transform: capitalize;
  }

  .status-indexed { color: #4ade80; }
  .status-embedding { color: #fbbf24; }
  .status-queued { color: #facc15; }
  .status-failed { color: #f87171; }

  .spin-slow {
    animation: spin 2s linear infinite;
  }

  .actions {
    display: flex;
    gap: 0.25rem;
  }

  .action-btn {
    background: transparent;
    border: none;
    color: #9ca3af;
    cursor: pointer;
    padding: 0.25rem;
    border-radius: 0.25rem;
    transition: all 0.2s;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .action-btn:hover:not(:disabled) {
    background-color: #1e293b;
  }
  
  .action-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .delete-btn:hover {
    color: #f87171;
  }

  .loading-state, .empty-state, .error-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 2rem;
    text-align: center;
    height: 100%;
    color: #9ca3af;
    gap: 0.75rem;
  }

  .error-state {
    color: #f87171;
  }

  .retry-link {
    background: none;
    border: none;
    color: #60a5fa;
    text-decoration: underline;
    cursor: pointer;
    font-size: 0.875rem;
    padding: 0;
  }

  .sub-text {
    font-size: 0.75rem;
    color: #6b7280;
  }

  .spinner {
    width: 1.5rem;
    height: 1.5rem;
    border: 2px solid #3b82f6;
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 1s linear infinite;
  }

  .spinner-small {
    width: 1rem;
    height: 1rem;
    border: 2px solid #9ca3af;
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 1s linear infinite;
  }

  .spin {
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }
  
  .content::-webkit-scrollbar {
    width: 6px;
  }
  
  .content::-webkit-scrollbar-track {
    background: #1e293b; 
  }
  
  .content::-webkit-scrollbar-thumb {
    background: #475569; 
    border-radius: 3px;
  }
  
  .content::-webkit-scrollbar-thumb:hover {
    background: #64748b; 
  }

  .confirm-overlay {
    position: absolute;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 10;
  }

  .confirm-dialog {
    background: #1e293b;
    border: 1px solid #334155;
    border-radius: 0.75rem;
    padding: 1.5rem;
    max-width: 350px;
    width: 90%;
    text-align: center;
  }

  .confirm-text {
    color: #e5e7eb;
    font-size: 0.95rem;
    margin: 0 0 0.5rem 0;
  }

  .confirm-sub {
    color: #9ca3af;
    font-size: 0.8rem;
    margin: 0 0 1.25rem 0;
  }

  .confirm-actions {
    display: flex;
    gap: 0.75rem;
    justify-content: center;
  }

  .confirm-cancel-btn {
    background: #334155;
    color: #e5e7eb;
    border: none;
    border-radius: 0.5rem;
    padding: 0.5rem 1.25rem;
    font-size: 0.875rem;
    cursor: pointer;
    transition: background 0.2s;
  }

  .confirm-cancel-btn:hover {
    background: #475569;
  }

  .confirm-delete-btn {
    background: #dc2626;
    color: white;
    border: none;
    border-radius: 0.5rem;
    padding: 0.5rem 1.25rem;
    font-size: 0.875rem;
    cursor: pointer;
    transition: background 0.2s;
  }

  .confirm-delete-btn:hover {
    background: #b91c1c;
  }
</style>
