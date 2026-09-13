<script lang="ts">
  import { onDestroy, tick } from 'svelte';
  import { pdfjsLib } from '$lib/pdf-init';
  import type { PDFDocumentLoadingTask, PDFDocumentProxy } from 'pdfjs-dist';
  import PdfPage from './PdfPage.svelte';
  import 'pdfjs-dist/web/pdf_viewer.css';
  import { errorDialog } from '$lib/stores/error-dialog.svelte';

  let {
    url = '',
    page = 1,
    contextPages = 3
  } = $props<{
    url?: string;
    page?: number;
    contextPages?: number;
  }>();

  let pdfDoc: PDFDocumentProxy | null = $state(null);
  let loadingTask: PDFDocumentLoadingTask | null = null;
  let loadingTaskDisposal: Promise<void> = Promise.resolve();
  let pageNum = $state(1);
  let scale = $state(1.0);
  let totalPages = $state(0);
  let loading = $state(true);
  let error: string | null = $state(null);
  let loadId = 0;
  let lastIncomingPage: number | null = null;

  let pagesToRender = $derived.by(() => {
    if (!totalPages) return [] as number[];
    const start = Math.max(1, pageNum - contextPages);
    const end = Math.min(totalPages, pageNum + contextPages);
    const result: number[] = [];
    for (let i = start; i <= end; i++) {
      result.push(i);
    }
    return result;
  });

  $effect(() => {
    if (url && pdfjsLib) {
      void loadDocument(url);
    }
  });

  $effect(() => {
    if (pdfDoc && page >= 1 && page <= totalPages && page !== lastIncomingPage) {
      pageNum = page;
      lastIncomingPage = page;
    }
  });

  $effect(() => {
    if (totalPages && !loading) {
      tick().then(() => {
        setTimeout(() => scrollToPage(pageNum), 100);
      });
    }
  });

  function scrollToPage(p: number) {
    const el = document.getElementById(`pdf-page-${p}`);
    el?.scrollIntoView({ behavior: 'smooth', block: 'center' });
  }


  function reportPdfError(action: () => void) {
    errorDialog.report({
      dedupeKey: 'pdf-rendering',
      title: 'Document preview unavailable',
      message: 'Ragtag could not display this PDF page.',
      guidance: 'Try again. If the problem continues, remove and upload a new copy of the document.',
      action: { label: 'Retry preview', run: action }
    });
  }

  function disposeLoadingTask(): Promise<void> {
    const task = loadingTask;
    loadingTask = null;
    if (!task) return loadingTaskDisposal;

    loadingTaskDisposal = loadingTaskDisposal.then(async () => {
      try {
        await task.destroy();
      } catch {
        // A task canceled by a newer preview must not trigger the global error dialog.
      }
    });
    return loadingTaskDisposal;
  }

  async function loadDocument(docUrl: string) {
    const myLoadId = ++loadId;
    loading = true;
    error = null;
    pdfDoc = null;
    totalPages = 0;
    await tick();
    if (myLoadId !== loadId) return;

    await disposeLoadingTask();
    if (myLoadId !== loadId) return;

    let task: PDFDocumentLoadingTask | null = null;

    try {
      task = pdfjsLib.getDocument({ url: docUrl, withCredentials: false });
      loadingTask = task;
      const document = await task.promise;
      if (myLoadId !== loadId) return;

      pdfDoc = document;
      totalPages = document.numPages;
      pageNum = (page && page <= totalPages) ? page : 1;
      loading = false;
      await tick();
      scrollToPage(pageNum);
    } catch {
      if (myLoadId !== loadId) return;

      await disposeLoadingTask();
      if (myLoadId !== loadId) return;

      error = 'Document preview unavailable.';
      loading = false;
      reportPdfError(() => void loadDocument(docUrl));
    }
  }

  function onPrevPage() {
    if (pageNum <= 1) return;
    pageNum -= 1;
  }

  function onNextPage() {
    if (pageNum >= totalPages) return;
    pageNum += 1;
  }

  function onZoomIn() {
    scale += 0.2;
  }

  function onZoomOut() {
    if (scale <= 0.4) return;
    scale -= 0.2;
  }

  onDestroy(() => {
    ++loadId;
    void disposeLoadingTask();
  });
</script>

<div class="pdf-container">
  {#if error}
    <div class="error-state"><p>{error}</p></div>
  {:else}
    <div class="toolbar">
      <div class="group">
        <button class="tool-btn" onclick={onPrevPage} disabled={pageNum <= 1} aria-label="Previous page">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m15 18-6-6 6-6"/></svg>
        </button>
        <span class="page-info">Page {pageNum} of {totalPages}</span>
        <button class="tool-btn" onclick={onNextPage} disabled={pageNum >= totalPages} aria-label="Next page">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>
        </button>
      </div>

      <div class="group">
        <button class="tool-btn" onclick={onZoomOut} disabled={scale <= 0.4} aria-label="Zoom out">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/><line x1="8" y1="11" x2="14" y2="11"/></svg>
        </button>
        <span class="zoom-info">{Math.round(scale * 100)}%</span>
        <button class="tool-btn" onclick={onZoomIn} aria-label="Zoom in">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/><line x1="11" y1="8" x2="11" y2="14"/><line x1="8" y1="11" x2="14" y2="11"/></svg>
        </button>
      </div>
    </div>

    <div class="scroll-container">
      {#if loading}
        <div class="loading-overlay"><div class="spinner"></div></div>
      {:else if pdfDoc}
        <div class="pages-list">
          {#each pagesToRender as p (p)}
            {@const isCited = p === page}
            <PdfPage
              {pdfDoc}
              pageNumber={p}
              {scale}
              isCitation={isCited}
              onError={() => reportPdfError(() => void loadDocument(url))}
            />
          {/each}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .pdf-container {
    display: flex;
    flex-direction: column;
    height: 100%;
    width: 100%;
    background-color: #0f172a;
    position: relative;
  }

  .toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.75rem 1.5rem;
    background-color: #0f172a;
    border-bottom: 1px solid #1e293b;
    color: #f1f5f9;
    z-index: 10;
    flex-shrink: 0;
  }

  .scroll-container {
    flex: 1;
    overflow: auto;
    position: relative;
    padding: 2rem;
    background-color: #0f172a;
  }

  .group {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .tool-btn {
    background: transparent;
    border: none;
    color: #f1f5f9;
    cursor: pointer;
    padding: 0.25rem;
    border-radius: 0.25rem;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: background-color 0.2s;
  }

  .tool-btn:hover:not(:disabled) {
    background-color: rgba(255,255,255,0.1);
  }

  .tool-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .page-info, .zoom-info {
    font-size: 0.875rem;
    font-variant-numeric: tabular-nums;
    min-width: 3rem;
    text-align: center;
  }

  .pages-list {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 1rem;
    min-height: 100%;
  }

  .loading-overlay {
    position: absolute;
    top: 0; left: 0; right: 0; bottom: 0;
    background-color: rgba(0,0,0,0.5);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .spinner {
    width: 3rem;
    height: 3rem;
    border: 4px solid #f3f3f3;
    border-top: 4px solid #3b82f6;
    border-radius: 50%;
    animation: spin 1s linear infinite;
  }

  .error-state {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: #fca5a5;
  }

  @keyframes spin {
    0% { transform: rotate(0deg); }
    100% { transform: rotate(360deg); }
  }
</style>
