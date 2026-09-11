export {};

// Static imports would evaluate rune modules before the shim exists in this Node-only test.
const runtime = globalThis as { $state?: <T>(value: T) => T };
runtime.$state = <T>(value: T) => value;

const { api } = await import('../api.ts');
const originalSetSetting = api.setSetting;
api.setSetting = async () => {
  throw new Error('storage unavailable');
};

const { SettingsState } = await import('./settings.svelte.ts');
const settings = new SettingsState();
settings.chunk_size = 64;

try {
  const saved = await settings.update('chunk_size', '64');
  if (saved || settings.chunk_size !== 512) {
    throw new Error('A failed bound setting update must restore its persisted value.');
  }
} finally {
  api.setSetting = originalSetSetting;
}
