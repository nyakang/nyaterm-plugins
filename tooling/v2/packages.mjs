import { createHash } from 'node:crypto';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

export async function downloadArchive(artifact) {
  let url = new URL(artifact.url);
  const signal = AbortSignal.timeout(120000);
  for (let attempt = 0; attempt < 6; attempt++) {
    if (url.protocol !== 'https:' || url.username || url.password || url.hash) throw new Error('Invalid archive URL');
    const response = await fetch(url, { redirect: 'manual', signal });
    if ([301, 302, 303, 307, 308].includes(response.status)) {
      await response.body?.cancel();
      const location = response.headers.get('location');
      if (!location) throw new Error('Missing redirect target');
      url = new URL(location, url);
      continue;
    }
    if (!response.ok || !response.body) throw new Error('Archive download failed');
    const chunks = []; let size = 0;
    for await (const chunk of response.body) {
      size += chunk.length;
      if (size > artifact.size) throw new Error('Archive exceeds reviewed size');
      chunks.push(chunk);
    }
    const bytes = Buffer.concat(chunks);
    if (size !== artifact.size || createHash('sha256').update(bytes).digest('hex') !== artifact.sha256) throw new Error('Archive differs from reviewed digest');
    return bytes;
  }
  throw new Error('Too many archive redirects');
}

export function inspectArchive(binary, archive, listing, release) {
  const result = spawnSync(binary, ['inspect', archive], { encoding: 'utf8', windowsHide: true, maxBuffer: 2 * 1024 * 1024 });
  if (result.error || result.status !== 0) throw new Error('Reviewed Rust inspector rejected archive');
  const inspected = JSON.parse(result.stdout);
  const manifest = inspected.manifest;
  if (inspected.protocol_version !== 2 || manifest.schema_version !== 2 || manifest.id !== listing.id || manifest.publisher !== listing.publisher
    || manifest.version !== release.version || manifest.api_version !== release.api_version || manifest.host_version !== release.host_version
    || JSON.stringify(manifest.capabilities) !== JSON.stringify(release.capabilities)) throw new Error('Archive manifest differs from catalog');
}

export async function verifyPackages(catalog, binary, key) {
  for (const listing of catalog.extensions) for (const release of listing.releases) {
    if (!key) throw new Error('NYATERM_EXTENSION_PUBLIC_KEY is required for package verification');
    const temporary = mkdtempSync(path.join(tmpdir(), 'nyaterm-store-validate-'));
    try {
      const archive = path.join(temporary, 'extension.zip');
      writeFileSync(archive, await downloadArchive(release.artifact));
      writeFileSync(`${archive}.minisig`, release.artifact.signature);
      const verified = spawnSync('minisign', ['-V', '-P', key, '-m', archive, '-x', `${archive}.minisig`], { windowsHide: true, encoding: 'utf8' });
      if (verified.error || verified.status !== 0) throw new Error('Archive signature verification failed');
      inspectArchive(binary, archive, listing, release);
    } finally { rmSync(temporary, { recursive: true, force: true }); }
  }
}
