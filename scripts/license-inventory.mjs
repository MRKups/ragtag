import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const inventoryPath = path.join(root, 'docs/third-party-license-inventory.json');
const inventory = JSON.parse(readFileSync(inventoryPath, 'utf8'));
const metadata = JSON.parse(execFileSync('cargo', [
  'metadata', '--locked', '--format-version', '1',
  '--manifest-path', 'src-tauri/Cargo.toml',
], { cwd: root, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 }));
const npmLock = JSON.parse(readFileSync(path.join(root, 'frontend/package-lock.json'), 'utf8'));
const compare = (a, b) => a < b ? -1 : a > b ? 1 : 0;

inventory.generated_on = new Date().toISOString().slice(0, 10);
inventory.license_data.note = 'Values are raw package declarations, not license approval. Missing declarations require review. Slash-separated values are not normalized SPDX expressions.';
inventory.regeneration = {
  when: 'Refresh after either input lockfile or package metadata changes and before a release.',
  command: 'npm run licenses:generate',
  rust: 'Resolve the locked package set with cargo metadata; requires cached sources or network access.',
  npm: 'Read every resolved frontend/package-lock.json package, including optional and development dependencies.',
};
inventory.components.rust = metadata.packages.map((pkg) => {
  const component = { name: pkg.name, version: pkg.version, license: pkg.license, source: pkg.source };
  if (!pkg.source) component.path = path.relative(root, path.dirname(pkg.manifest_path)).split(path.sep).join('/');
  return component;
}).sort((a, b) => compare(a.name, b.name) || compare(a.version, b.version) || compare(a.source ?? '', b.source ?? ''));
inventory.components.npm = Object.entries(npmLock.packages)
  .filter(([entry]) => entry !== '')
  .map(([entry, pkg]) => ({
    path: entry,
    name: pkg.name ?? entry.split('node_modules/').at(-1),
    version: pkg.version,
    license: pkg.license ?? null,
    resolved: pkg.resolved ?? null,
    integrity: pkg.integrity ?? null,
    dev: pkg.dev ?? false,
    optional: pkg.optional ?? false,
    os: pkg.os ?? null,
    cpu: pkg.cpu ?? null,
  }));
for (const kind of ['rust', 'npm']) {
  const source = inventory.sources[kind];
  source.lockfile_sha256 = createHash('sha256').update(readFileSync(path.join(root, source.lockfile))).digest('hex');
  source.package_count = inventory.components[kind].length;
}
inventory.sources.rust.resolution_command = 'cargo metadata --locked --format-version 1 --manifest-path src-tauri/Cargo.toml';
writeFileSync(inventoryPath, `${JSON.stringify(inventory, null, 2)}\n`);
console.log(`Inventoried ${inventory.sources.rust.package_count} Rust and ${inventory.sources.npm.package_count} npm packages.`);
