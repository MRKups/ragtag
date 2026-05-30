import * as pdfjsLib from 'pdfjs-dist';

// Initialize the worker once using a persistent Worker instance.
// This prevents the worker from being terminated when documents are destroyed,
// avoiding repeated downloads of the worker script and improving performance.
if (typeof window !== 'undefined' && !pdfjsLib.GlobalWorkerOptions.workerPort) {
  const worker = new Worker('/pdf.worker.mjs', { type: 'module' });
  pdfjsLib.GlobalWorkerOptions.workerPort = worker;
}

export { pdfjsLib };
