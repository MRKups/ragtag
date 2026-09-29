export {};

// Static imports would evaluate rune modules before the shim exists in this Node-only test.
const runtime = globalThis as { $state?: <T>(value: T) => T };
runtime.$state = <T>(value: T) => value;

const { api } = await import('../api.ts');
const originalSetSetting = api.setSetting;
const { SettingsState } = await import('./settings.svelte.ts');

try {
  api.setSetting = async () => {
    throw new Error('storage unavailable');
  };

  const failedUpdate = new SettingsState();
  failedUpdate.chunk_size = 64;
  const saved = await failedUpdate.update('chunk_size', '64');
  if (saved || failedUpdate.chunk_size !== 512) {
    throw new Error('A failed bound setting update must restore its persisted value.');
  }

  api.setSetting = async () => {
    throw { kind: 'Database', message: 'QueueFull was mentioned by the database.' };
  };

  const rejectedNonQueueFull = new SettingsState();
  rejectedNonQueueFull.chunk_size = 64;
  const nonQueueFullSaved = await rejectedNonQueueFull.update('chunk_size', '64');
  if (nonQueueFullSaved || rejectedNonQueueFull.chunk_size !== 512) {
    throw new Error('Only the typed QueueFull error may commit a chunk setting.');
  }

  api.setSetting = async () => {
    throw { kind: 'QueueFull', message: 'The reindex queue is full.' };
  };

  const queuedReindex = new SettingsState();
  queuedReindex.chunk_size = 64;
  const queueFullSaved = await queuedReindex.update('chunk_size', '64');
  if (!queueFullSaved || queuedReindex.chunk_size !== 64) {
    throw new Error('A QueueFull chunk update must retain its committed setting value.');
  }

  let rejectQueueFull: (reason?: unknown) => void = () => {
    throw new Error('The deferred QueueFull request was never initialized.');
  };
  api.setSetting = () => {
    return new Promise<void>((_, reject) => {
      rejectQueueFull = reject;
    });
  };

  const newerChunkDraft = new SettingsState();
  newerChunkDraft.chunk_size = 64;
  const staleQueueFull = newerChunkDraft.update('chunk_size', '64');
  newerChunkDraft.chunk_size = 96;
  rejectQueueFull({ kind: 'QueueFull', message: 'The reindex queue is full.' });
  if (!(await staleQueueFull) || newerChunkDraft.chunk_size !== 96) {
    throw new Error('A QueueFull response must not overwrite a newer unsaved chunk draft.');
  }


  const inferenceRequests: Array<{ key: string; value: string; resolve: () => void }> = [];
  api.setSetting = (key, value) => {
    return new Promise<void>((resolve) => {
      inferenceRequests.push({ key, value, resolve });
    });
  };

  const inferenceMode = new SettingsState();
  const selectingExternal = inferenceMode.updateInferenceMode('external');
  const selectingLocal = inferenceMode.updateInferenceMode('local');
  if (
    inferenceRequests.length !== 1
    || inferenceRequests[0].key !== 'inference_mode'
    || inferenceRequests[0].value !== 'external'
  ) {
    throw new Error('Inference mode changes must wait for earlier selections.');
  }

  inferenceRequests[0].resolve();
  await selectingExternal;
  const localRequest = inferenceRequests[1];
  if (
    !localRequest
    || localRequest.key !== 'inference_mode'
    || localRequest.value !== 'local'
  ) {
    throw new Error('A rapid local reversal must remain queued after the remote selection.');
  }

  localRequest.resolve();
  await selectingLocal;
  if (inferenceMode.inference_mode !== 'local') {
    throw new Error('The latest queued inference mode must be the final persisted mode.');
  }
  let setSettingCalls = 0;
  api.setSetting = async () => {
    setSettingCalls += 1;
  };

  const blankInput = new SettingsState();
  blankInput.chunk_overlap = Number.NaN;
  const blankSaved = await blankInput.update('chunk_overlap', '');
  if (blankSaved || blankInput.chunk_overlap !== 80 || setSettingCalls !== 0) {
    throw new Error('A blank numeric chunk input must restore its persisted value without saving.');
  }

  const themeSettingCalls: Array<{ key: string; value: string }> = [];
  api.setSetting = async (key: string, value: string) => {
    themeSettingCalls.push({ key, value });
  };
  const themeState = new SettingsState();
  if ((themeState.theme as string) !== 'dark') {
    throw new Error('Default theme must be dark.');
  }
  await themeState.toggleTheme();
  if ((themeState.theme as string) !== 'light') {
    throw new Error('Toggling dark theme must switch to light.');
  }
  if (themeSettingCalls.length !== 1 || themeSettingCalls[0].key !== 'theme' || themeSettingCalls[0].value !== 'light') {
    throw new Error('Toggling theme must persist the new theme via api.setSetting.');
  }
  await themeState.toggleTheme();
  if ((themeState.theme as string) !== 'dark') {
    throw new Error('Toggling light theme must switch back to dark.');
  }
} finally {
  api.setSetting = originalSetSetting;
}
