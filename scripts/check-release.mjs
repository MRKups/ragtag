import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

export function releaseProblems(read) {
  const problems = [];
  const json = (file) => JSON.parse(read(file));
  const root = json('package.json');
  const frontend = json('frontend/package.json');
  const tauri = json('src-tauri/tauri.conf.json');
  const lock = json('frontend/package-lock.json');
  const inventory = json('docs/third-party-license-inventory.json');
  const manifests = ['src-tauri/Cargo.toml', ...['domain', 'persistence', 'rag_pipeline'].map((crate) => `src-tauri/crates/${crate}/Cargo.toml`)];
  const versions = [['package.json', root.version], ['frontend/package.json', frontend.version], ['Tauri bundle', tauri.version], ['frontend lockfile', lock.version], ['frontend lockfile root', lock.packages[''].version]];
  for (const file of manifests) {
    const manifest = read(file).toString();
    const pkg = manifest.split(/^\[package\]\s*$/m)[1]?.split(/^\[/m)[0] ?? '';
    const version = pkg.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
    versions.push([file, version]);
    if (!/^license\s*=\s*"MIT"\s*$/m.test(pkg)) problems.push(`${file} must declare MIT.`);
    const component = inventory.components.rust.find((component) => component.path === path.posix.dirname(file));
    if (!component || component.version !== version || component.license !== 'MIT') problems.push(`${file} inventory declaration is stale.`);
  }
  for (const [file, version] of versions) {
    if (!version || version !== tauri.version) problems.push(`${file} version ${version} differs from Tauri ${tauri.version}.`);
  }
  for (const [file, pkg] of [['package.json', root], ['frontend/package.json', frontend]]) {
    if (pkg.license !== 'MIT' || pkg.private !== true) problems.push(`${file} must declare MIT and private: true.`);
  }
  for (const kind of ['rust', 'npm']) {
    const source = inventory.sources[kind];
    const hash = createHash('sha256').update(read(source.lockfile)).digest('hex');
    if (source.lockfile_sha256 !== hash) problems.push(`${kind} license inventory is stale; run npm run licenses:generate.`);
    if (source.package_count !== inventory.components[kind].length) problems.push(`${kind} inventory count does not match its entries.`);
  }
  const npmComponents = inventory.components.npm;
  const npmEntries = Object.entries(lock.packages).filter(([entry]) => entry !== '');
  if (npmComponents.length !== npmEntries.length || npmEntries.some(([entry, pkg]) => !npmComponents.some((component) => component.path === entry && component.version === pkg.version && component.license === (pkg.license ?? null)))) {
    problems.push('npm inventory does not match locked package declarations.');
  }
  for (const file of ['LICENSE', 'THIRD_PARTY_NOTICES.md']) {
    if (!read(file).toString().trim()) problems.push(`${file} must not be empty.`);
  }
  return problems;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = fileURLToPath(new URL('../', import.meta.url));
  try {
    const problems = releaseProblems((file) => readFileSync(path.join(root, file)));
    if (problems.length) {
      console.error(problems.join('\n'));
      process.exitCode = 1;
    } else {
      console.log('Source-release metadata and lockfile inventory checks passed.');
    }
  } catch (error) {
    console.error(`Release check failed: ${error.message}`);
    process.exitCode = 1;
  }
}
