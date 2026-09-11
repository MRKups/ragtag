import * as pdfjsLib from 'pdfjs-dist/legacy/build/pdf.mjs';

// The generic PDF.js build requires Map.prototype.getOrInsertComputed, which
// macOS's JavaScriptCore does not provide. The legacy build supplies it.
if (typeof window !== 'undefined' && !pdfjsLib.GlobalWorkerOptions.workerPort) {
  const worker = new Worker(
    new URL('pdfjs-dist/legacy/build/pdf.worker.mjs', import.meta.url),
    { type: 'module' }
  );
  pdfjsLib.GlobalWorkerOptions.workerPort = worker;
}

export { pdfjsLib };
