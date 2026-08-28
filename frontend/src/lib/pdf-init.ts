import * as pdfjsLib from 'pdfjs-dist';

// Vite emits the matching worker from the installed PDF.js package.
if (typeof window !== 'undefined' && !pdfjsLib.GlobalWorkerOptions.workerPort) {
  const worker = new Worker(
    new URL('pdfjs-dist/build/pdf.worker.mjs', import.meta.url),
    { type: 'module' }
  );
  pdfjsLib.GlobalWorkerOptions.workerPort = worker;
}

export { pdfjsLib };
