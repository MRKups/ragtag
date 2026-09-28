<script lang="ts">
  import { onDestroy, tick } from 'svelte';
  import { pdfjsLib } from '$lib/pdf-init';
  import type {
    PDFDocumentProxy,
    PDFPageProxy,
    PageViewport,
    RenderTask,
    TextLayer
  } from 'pdfjs-dist';

  let {
    pdfDoc,
    pageNumber,
    scale = 1.0,
    isCitation = false,
    onError = () => {}
  } = $props<{
    pdfDoc: PDFDocumentProxy;
    pageNumber: number;
    scale?: number;
    isCitation?: boolean;
    onError?: () => void;
  }>();

  let canvas: HTMLCanvasElement;
  let textLayerDiv: HTMLDivElement;
  let width = $state(900);
  let height = $state(1200);
  let loading = $state(true);
  let renderTask: RenderTask | null = null;
  let activePage: PDFPageProxy | null = null;
  let renderId = 0;
  let textLayer: TextLayer | null = null;
  let mounted = $state(false);

  $effect(() => {
    mounted = true;
  });

  $effect(() => {
    if (mounted && pdfDoc && pageNumber && scale) {
      void renderPage();
    }
  });

  function cancelRender() {
    renderTask?.cancel();
    renderTask = null;
    textLayer?.cancel();
    textLayer = null;
    if (activePage) {
      activePage.cleanup();
      activePage = null;
    }
  }

  async function renderPage() {
    const myId = ++renderId;
    if (!pdfDoc || !canvas || !textLayerDiv) return;

    cancelRender();
    loading = true;
    await tick();
    if (myId !== renderId) return;

    let task: RenderTask | null = null;
    try {
      const page = await pdfDoc.getPage(pageNumber);
      if (myId !== renderId) {
        page.cleanup();
        return;
      }
      activePage = page;

      const dpr = window.devicePixelRatio || 1;
      const viewport = page.getViewport({ scale });
      width = viewport.width;
      height = viewport.height;

      canvas.width = Math.floor(viewport.width * dpr);
      canvas.height = Math.floor(viewport.height * dpr);
      canvas.style.width = `${Math.floor(viewport.width)}px`;
      canvas.style.height = `${Math.floor(viewport.height)}px`;

      const currentTask = page.render({
        canvas,
        viewport,
        transform: dpr === 1 ? undefined : [dpr, 0, 0, dpr, 0, 0]
      });
      task = currentTask;
      renderTask = currentTask;
      await currentTask.promise;
      if (myId !== renderId) return;
      if (renderTask === currentTask) renderTask = null;

      loading = false;
      await renderTextLayer(page, viewport, myId);
    } catch (error: unknown) {
      if (
        myId !== renderId
        || (error instanceof Error && error.name === 'RenderingCancelledException')
      ) {
        return;
      }

      if (renderTask === task) renderTask = null;
      loading = false;
      console.error(`Error rendering PDF page ${pageNumber}:`, error);
      onError();
    }
  }

  async function renderTextLayer(page: PDFPageProxy, viewport: PageViewport, myId: number) {
    textLayerDiv.replaceChildren();
    textLayerDiv.style.setProperty('--total-scale-factor', `${scale}`);
    const layer = new pdfjsLib.TextLayer({
      textContentSource: page.streamTextContent(),
      container: textLayerDiv,
      viewport
    });
    textLayer = layer;

    try {
      await layer.render();
    } catch (error) {
      if (myId === renderId) {
        textLayerDiv.replaceChildren();
        console.warn(`Text selection unavailable for PDF page ${pageNumber}:`, error);
      }
    } finally {
      if (textLayer === layer) textLayer = null;
    }
  }

  onDestroy(() => {
    ++renderId;
    cancelRender();
  });
</script>

<div
  class="page-wrapper {isCitation ? 'citation-page' : ''}"
  style="width: {width}px; height: {height}px;"
>
  <div class="canvas-wrapper">
    <canvas bind:this={canvas}></canvas>
  </div>
  <div class="textLayer" bind:this={textLayerDiv}></div>

  {#if isCitation}
    <div class="citation-label">Cited Page</div>
  {/if}

  {#if loading}
    <div class="loading-overlay"><div class="spinner"></div></div>
  {/if}

  <div class="page-number">Page {pageNumber}</div>
</div>

<style>
  .page-wrapper {
    position: relative;
    margin: 0 auto;
    background-color: white;
    box-shadow: 0 0 15px rgba(0, 0, 0, 0.5);
    border-radius: 4px;
    transition: transform 0.2s ease;
    min-height: 200px;
  }

  .citation-page {
    box-shadow: 0 0 0 4px rgba(59, 130, 246, 0.6), 0 25px 50px -12px rgba(0, 0, 0, 0.5);
    z-index: 10;
    transform: scale(1.02);
    border: 2px solid #3b82f6;
  }

  .citation-label {
    position: absolute;
    top: -1.75rem;
    left: 50%;
    transform: translateX(-50%);
    background-color: #3b82f6;
    color: white;
    padding: 0.25rem 0.75rem;
    border-radius: 9999px;
    font-size: 0.75rem;
    font-weight: 600;
    box-shadow: 0 2px 4px rgba(0,0,0,0.2);
    white-space: nowrap;
  }

  .canvas-wrapper {
    position: absolute;
    top: 0; left: 0;
    width: 100%; height: 100%;
    overflow: hidden;
    border-radius: 4px;
  }

  canvas {
    display: block;
    width: 100%;
    height: 100%;
  }

  .textLayer {
    position: absolute;
    left: 0; top: 0; right: 0; bottom: 0;
    overflow: hidden;
    opacity: 1;
    line-height: 1.0;
  }

  .textLayer :global(span) {
    color: transparent;
    position: absolute;
    white-space: pre;
    cursor: text;
    transform-origin: 0% 0%;
  }

  .textLayer :global(::selection) {
    background: rgba(37, 99, 235, 0.5) !important;
    color: transparent;
  }

  .textLayer :global(br) {
    display: none;
  }

  .loading-overlay {
    position: absolute;
    top: 0; left: 0; right: 0; bottom: 0;
    background: rgba(255, 255, 255, 0.8);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 20;
  }

  .spinner {
    width: 2rem;
    height: 2rem;
    border: 3px solid #f3f3f3;
    border-top: 3px solid #3b82f6;
    border-radius: 50%;
    animation: spin 1s linear infinite;
  }

  .page-number {
    position: absolute;
    bottom: -1.5rem;
    left: 50%;
    transform: translateX(-50%);
    color: #9ca3af;
    font-size: 0.75rem;
  }

  @keyframes spin {
    0% { transform: rotate(0deg); }
    100% { transform: rotate(360deg); }
  }
</style>
