import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { test } from 'node:test';
import { releaseProblems } from './check-release.mjs';

const root = new URL('../', import.meta.url);
const read = (file) => readFileSync(new URL(file, root));
const withJson = (target, update) => (file) => {
  const content = read(file);
  if (file !== target) return content;
  const data = JSON.parse(content);
  update(data);
  return Buffer.from(JSON.stringify(data));
};

test('current release inputs are consistent', () => {
  assert.deepEqual(releaseProblems(read), []);
});

test('source checks do not require community policies or GitHub templates', () => {
  assert.deepEqual(releaseProblems((file) => {
    assert(!['SECURITY.md', 'CODE_OF_CONDUCT.md', 'CONTRIBUTING.md'].includes(file));
    assert(!file.startsWith('.github/'));
    return read(file);
  }), []);
});

test('rejects mismatched versions and accidental npm publishing', () => {
  const problems = releaseProblems(withJson('package.json', (pkg) => {
    pkg.version = '99.0.0';
    pkg.private = false;
  }));
  assert(problems.some((message) => message.includes('version 99.0.0')));
  assert(problems.some((message) => message.includes('private: true')));
});

test('rejects stale inventory hashes and incorrect counts', () => {
  const problems = releaseProblems(withJson('docs/third-party-license-inventory.json', (inventory) => {
    inventory.sources.rust.lockfile_sha256 = '0'.repeat(64);
    inventory.sources.npm.package_count += 1;
  }));
  assert(problems.some((message) => message.includes('rust license inventory is stale')));
  assert(problems.some((message) => message.includes('npm inventory count')));
});

test('rejects stale first-party Rust license declarations', () => {
  const problems = releaseProblems(withJson('docs/third-party-license-inventory.json', (inventory) => {
    inventory.components.rust.find((component) => component.path === 'src-tauri').license = null;
  }));
  assert(problems.some((message) => message.includes('src-tauri/Cargo.toml inventory declaration is stale')));
});

test('rejects edited npm declarations even with a matching hash', () => {
  const lock = JSON.parse(read('frontend/package-lock.json'));
  const entry = Object.keys(lock.packages).find((entry) => entry !== '');
  lock.packages[entry].license = 'UNKNOWN';
  const bytes = Buffer.from(JSON.stringify(lock));
  const inventory = JSON.parse(read('docs/third-party-license-inventory.json'));
  inventory.sources.npm.lockfile_sha256 = createHash('sha256').update(bytes).digest('hex');
  const problems = releaseProblems((file) => {
    if (file === 'frontend/package-lock.json') return bytes;
    if (file === 'docs/third-party-license-inventory.json') return Buffer.from(JSON.stringify(inventory));
    return read(file);
  });
  assert(problems.some((message) => message.includes('npm inventory does not match')));
});
