import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { buildCatalog, read, root } from './catalog.mjs';

const operation = process.argv[2];
if (operation === 'inspect') {
  const binary = process.env.NYATERM_PLUGIN_CLI;
  if (!binary || !process.argv[3]) throw new Error('Set NYATERM_PLUGIN_CLI and provide an archive');
  const result = spawnSync(binary, ['inspect', process.argv[3]], { encoding: 'utf8', windowsHide: true, maxBuffer: 2 * 1024 * 1024 });
  if (result.error || result.status !== 0) throw new Error('Trusted package inspection failed');
  const inspection = JSON.parse(result.stdout);
  if (inspection.protocol_version !== 2 || inspection.manifest.schema_version !== 2) throw new Error('Expected V2 inspector');
  process.stdout.write(JSON.stringify(inspection) + '\n');
} else {
  const catalog = buildCatalog();
  const file = path.join(root, 'public/v2/plugins.json');
  if (operation === 'catalog') {
    mkdirSync(path.dirname(file), { recursive: true });
    writeFileSync(file, JSON.stringify(catalog, null, 2) + '\n');
  } else if (operation === 'validate') {
    if (JSON.stringify(read(file)) !== JSON.stringify(catalog)) throw new Error('Catalog drift; run npm run catalog');
    if (process.argv.includes('--packages')) {
      const binary = process.env.NYATERM_PLUGIN_CLI;
      if (!binary) throw new Error('Set NYATERM_PLUGIN_CLI to the reviewed V2 validator');
      const parsed = spawnSync(binary, ['catalog', file], { encoding: 'utf8', windowsHide: true, maxBuffer: 8 * 1024 * 1024 });
      if (parsed.error || parsed.status !== 0 || JSON.parse(parsed.stdout).protocol_version !== 2) throw new Error('Rust catalog contract rejected the generated catalog');
      const { verifyPackages } = await import('./packages.mjs');
      await verifyPackages(catalog, binary, process.env.NYATERM_EXTENSION_PUBLIC_KEY);
    }
    process.stdout.write(`Validated ${catalog.extensions.length} V2 extensions.\n`);
  } else throw new Error('Usage: cli.mjs catalog | validate | inspect <archive>');
}
