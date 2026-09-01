import { errorMessage } from './error.ts';

if (
  !errorMessage({
    kind: 'Inference',
    message: 'Inference error: No usable document excerpts were retrieved'
  }).includes('No usable document excerpts were retrieved')
) {
  throw new Error('Typed AppError payload was not classified as missing evidence.');
}

if (errorMessage(new Error('fallback error')) !== 'fallback error') {
  throw new Error('Native Error fallback was not preserved.');
}
