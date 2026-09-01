<script lang="ts">
  import { onDestroy, tick } from 'svelte';
  import { pdfjsLib } from '$lib/pdf-init';
  import type { PDFDocumentProxy } from 'pdfjs-dist';

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
  let renderTask: any = null;
  let renderId = 0;
  let mounted = $state(false);

  $effect(() => {
    mounted = true;
  });

  $effect(() => {
    if (mounted && pdfDoc && pageNumber && scale) {
      renderPage();
    }
  });

  async function renderPage() {
    const myId = ++renderId;
    if (!pdfDoc || !canvas || !textLayerDiv) return;
    loading = true;

    if (renderTask) {
      try { await renderTask.cancel(); } catch (_) {}
      renderTask = null;
    }

    await tick();
    if (myId !== renderId) return;

    try {
      const page = await pdfDoc.getPage(pageNumber);
      if (myId !== renderId) return;

      const dpr = window.devicePixelRatio || 1;
      const viewport = page.getViewport({ scale });

      width = viewport.width;
      height = viewport.height;

      canvas.width = Math.floor(viewport.width * dpr);
      canvas.height = Math.floor(viewport.height * dpr);
      canvas.style.width = `${Math.floor(viewport.width)}px`;
      canvas.style.height = `${Math.floor(viewport.height)}px`;

      const ctx = canvas.getContext('2d');
      if (!ctx) return;
      ctx.scale(dpr, dpr);

      renderTask = page.render({ canvasContext: ctx, viewport });
      await renderTask.promise;
      renderTask = null;

      // Text layer
      try {
        const textContent = await page.getTextContent();
        if (myId !== renderId) return;
        textLayerDiv.innerHTML = '';
        (textLayerDiv.style as any).setProperty('--scale-factor', `${scale}`);

        // @ts-ignore
        if (pdfjsLib.TextLayer) {
          // @ts-ignore
          const textLayer = new pdfjsLib.TextLayer({
            textContentSource: textContent,
            container: textLayerDiv,
            viewport
          });
          await textLayer.render();
        }
      } catch {
        if (myId === renderId) onError();
      }

      loading = false;
    } catch (e: any) {
      if (e.name !== 'RenderingCancelledException') {
        loading = false;
        console.error(`Error rendering page ${pageNumber}:`, e);
        onError();
      }
    }
  }

  onDestroy(() => {
    if (renderTask) {
      try { renderTask.cancel(); } catch (_) {}
    }
  });
</script>

<div
  id="pdf-page-{pageNumber}"
  class="page-wrapper {isCitation ? 'citation-page' : ''}"
  style="width: {width}px; height: {height}px;"
>
  <div class="canvas-wrapper">
    <canvas bind:this={canvas}></canvas>
  </div>
  <div class="text-layer" bind:this={textLayerDiv}></div>

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
    margin: 2rem auto;
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

  .text-layer {
    position: absolute;
    left: 0; top: 0; right: 0; bottom: 0;
    overflow: hidden;
    opacity: 1;
    line-height: 1.0;
  }

  .text-layer :global(span) {
    color: transparent;
    position: absolute;
    white-space: pre;
    cursor: text;
    transform-origin: 0% 0%;
  }

  .text-layer :global(::selection) {
    background: rgba(37, 99, 235, 0.5) !important;
    color: transparent;
  }

  .text-layer :global(br) {
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
