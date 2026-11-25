import test from 'node:test';
import assert from 'node:assert/strict';
import { validateListing } from './catalog.mjs';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const fixture = () => ({ id: 'example', name: 'Example', description: '', publisher: 'nyakang', releases: [{
  version: '1.0.0', host_version: '>=2.0.8', api_version: '2.0.0', capabilities: ['network'],
  artifact: { url: 'https://example.com/a.zip', sha256: 'a'.repeat(64), size: 1, signature: 'fixture' },
}] });
test('accepts V2 capabilities and rejects old permissions', () => {
  assert.equal(validateListing(fixture()).id, 'example');
  const old = fixture(); old.releases[0].capabilities = ['terminal.execute'];
  assert.throws(() => validateListing(old));
});
test('rejects reserved helpers, duplicate releases and insecure URLs', () => {
  const reserved = fixture(); reserved.id = 'nyaterm-native-rdp';
  assert.throws(() => validateListing(reserved));
  const duplicate = fixture(); duplicate.releases.push(duplicate.releases[0]);
  assert.throws(() => validateListing(duplicate));
  const insecure = fixture(); insecure.releases[0].artifact.url = 'http://example.com/a.zip';
  assert.throws(() => validateListing(insecure));
});

test('rejects unknown fields, duplicate capabilities, credentials and fragments', () => {
  for (const mutate of [
    value => value.v1 = true,
    value => value.id = "con",
    value => value.publisher = "lpt1",
    value => value.releases[0].permissions = [],
    value => value.releases[0].capabilities.push('network'),
    value => value.releases[0].artifact.url = 'https://user:pass@example.com/a.zip',
    value => value.releases[0].artifact.url += '#ignored',
    value => value.releases[0].artifact.size = 67108865,
    value => value.releases[0].artifact.sha256 = 'A'.repeat(64),
  ]) {
    const value = fixture(); mutate(value); assert.throws(() => validateListing(value));
  }
});

test('nonempty generated catalog uses the actual Rust client contract', () => {
  const catalog = { catalog_version: 2, extensions: [validateListing(fixture())] };
  if (process.env.NYATERM_TEST_CATALOG_OUTPUT) writeFileSync(process.env.NYATERM_TEST_CATALOG_OUTPUT, JSON.stringify(catalog));
  const binary = process.env.NYATERM_PLUGIN_CLI;
  if (!binary) return;
  const directory = mkdtempSync(path.join(tmpdir(), 'nyaterm-catalog-test-'));
  try {
    const file = path.join(directory, 'catalog.json');
    writeFileSync(file, JSON.stringify(catalog));
    const result = spawnSync(binary, ['catalog', file], { encoding: 'utf8', windowsHide: true });
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(JSON.parse(result.stdout), { protocol_version: 2, catalog });
    catalog.extensions[0].releases[0].artifact.url = 'https://user:secret@example.com/a.zip';
    writeFileSync(file, JSON.stringify(catalog));
    assert.notEqual(spawnSync(binary, ['catalog', file], { windowsHide: true }).status, 0);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});
