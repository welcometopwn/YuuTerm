import { execFileSync } from 'node:child_process';
import { readFileSync, readdirSync, mkdirSync, copyFileSync, writeFileSync, existsSync } from 'node:fs';
import path from 'node:path';

const out = path.resolve('target/third-party');
mkdirSync(out, { recursive: true });
const inventory = [];
function collect(root, destination, depth = 0) {
  if (depth > 3) return;
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    if (entry.isSymbolicLink()) continue;
    const source = path.join(root, entry.name);
    if (entry.isDirectory()) {
      if (!['node_modules', '.git', 'target', 'tests', 'test', 'examples'].includes(entry.name))
        collect(source, path.join(destination, entry.name), depth + 1);
    } else if (/^(licen[sc]e|copying|copyright|notice|ofl|unlicense)([._-]|$)/i.test(entry.name)) {
      mkdirSync(destination, { recursive: true });
      copyFileSync(source, path.join(destination, entry.name));
    }
  }
}
const metadata = JSON.parse(execFileSync('cargo', ['metadata', '--locked', '--format-version', '1', '--filter-platform', 'x86_64-pc-windows-msvc'], { maxBuffer: 32 * 1024 * 1024 }));
for (const pkg of metadata.packages) {
  if (!pkg.source) continue;
  collect(path.dirname(pkg.manifest_path), path.join(out, 'rust', `${pkg.name}-${pkg.version}`));
  inventory.push({ ecosystem: 'cargo', name: pkg.name, version: pkg.version, license: pkg.license, repository: pkg.repository });
}
const lock = JSON.parse(readFileSync('package-lock.json', 'utf8'));
for (const [location, item] of Object.entries(lock.packages)) {
  if (!location || !existsSync(location)) continue;
  const manifest = path.join(location, 'package.json');
  if (!existsSync(manifest)) continue;
  const pkg = JSON.parse(readFileSync(manifest, 'utf8'));
  collect(location, path.join(out, 'npm', `${pkg.name.replaceAll('/', '_')}-${pkg.version}`));
  inventory.push({ ecosystem: 'npm', name: pkg.name, version: pkg.version, license: pkg.license ?? item.license ?? null });
}
mkdirSync(path.join(out, 'fonts'), { recursive: true });
copyFileSync('ui/assets/fonts/OFL.txt', path.join(out, 'fonts/OFL.txt'));
writeFileSync(path.join(out, 'inventory.json'), JSON.stringify(inventory, null, 2) + '\n');
console.log(`Collected third-party notices for ${inventory.length} dependencies`);
