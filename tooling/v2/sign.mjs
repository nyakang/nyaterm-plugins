import { createHash } from 'node:crypto';
import { mkdirSync, writeFileSync, readFileSync, existsSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { root, read, buildCatalog, validateListing } from './catalog.mjs';

const [id, version, output] = process.argv.slice(2);
if (!/^[a-z][a-z0-9_-]{0,95}$/.test(id ?? '') || !/^[0-9A-Za-z.+-]+$/.test(version ?? '') || !output) throw new Error('Usage: sign.mjs <id> <version> <output-directory>');
const cli = process.env.NYATERM_PLUGIN_CLI;
const secret = process.env.NYATERM_STORE_KEY_FILE;
const publicKey = process.env.NYATERM_EXTENSION_PUBLIC_KEY;
if (!cli || !secret || !publicKey) throw new Error('Configure reviewed validator, Minisign key file and public key');
const candidate = read(path.join(root, 'extensions-v2', id, 'submissions', `${version}.json`));
const listingPath = path.join(root, 'extensions-v2', id, 'listing.json');
const listing = validateListing(read(listingPath));
if (listing.releases.some(release => release.version === version)) throw new Error('Published versions are immutable');
if (!Number.isSafeInteger(candidate.size) || candidate.size < 1 || candidate.size > 64 * 1024 * 1024 || !/^[a-f0-9]{64}$/.test(candidate.sha256)) throw new Error('Invalid candidate bounds');
const revoked = read(path.join(root, 'trust/revoked.json'));
if (revoked.artifacts.includes(candidate.sha256)) throw new Error('Candidate digest is revoked');
let url = new URL(candidate.url);
let response;
const deadline = AbortSignal.timeout(120000);
for (let redirects = 0; redirects <= 5; redirects++) {
  if (url.protocol !== 'https:' || url.username || url.password || url.hash) throw new Error('Candidate requires HTTPS without credentials');
  response = await fetch(url, { redirect: 'manual', signal: deadline });
  if ([301,302,303,307,308].includes(response.status)) {
    await response.body?.cancel(); url = new URL(response.headers.get('location'), url); continue;
  }
  break;
}
if (!response?.ok || !response.body) throw new Error('Candidate download failed');
const chunks = []; let size = 0;
for await (const chunk of response.body) {
  size += chunk.length;
  if (size > candidate.size) throw new Error('Candidate exceeds reviewed size');
  chunks.push(chunk);
}
const bytes = Buffer.concat(chunks);
if (size !== candidate.size || createHash('sha256').update(bytes).digest('hex') !== candidate.sha256) throw new Error('Candidate differs from reviewed archive');
mkdirSync(output, { recursive: true });
const asset = `${id}-${version}.zip`;
const archive = path.resolve(output, asset);
writeFileSync(archive, bytes, { flag: 'wx' });
const inspected = spawnSync(cli, ['inspect', archive], { encoding: 'utf8', windowsHide: true, maxBuffer: 2 * 1024 * 1024 });
if (inspected.status !== 0) throw new Error('Trusted validator rejected candidate');
const result = JSON.parse(inspected.stdout);
const manifest = result.manifest;
if (result.protocol_version !== 2 || manifest.id !== id || manifest.version !== version || manifest.publisher !== listing.publisher
  || JSON.stringify(manifest.capabilities) !== JSON.stringify(candidate.capabilities)
  || manifest.api_version !== candidate.api_version || manifest.host_version !== candidate.host_version) throw new Error('Manifest differs from reviewed candidate');
const signatureFile = `${archive}.minisig`;
if (existsSync(signatureFile)) throw new Error('Signature already exists');
const signed = spawnSync('minisign', ['-S', '-s', secret, '-m', archive, '-x', signatureFile], { stdio: 'inherit', windowsHide: true });
if (signed.status !== 0) throw new Error('Minisign signing failed');
const verified = spawnSync('minisign', ['-V', '-P', publicKey, '-m', archive, '-x', signatureFile], { stdio: 'inherit', windowsHide: true });
if (verified.status !== 0) throw new Error('Signature does not match client trust key');
const tag = `${id}-v${version}`;
listing.releases.push({ version, host_version: manifest.host_version, api_version: manifest.api_version, capabilities: manifest.capabilities,
  artifact: { url: `https://github.com/nyakang/nyaterm-plugins/releases/download/${tag}/${asset}`, sha256: candidate.sha256, size,
    signature: readFileSync(signatureFile, 'utf8') } });
validateListing(listing);
writeFileSync(listingPath, JSON.stringify(listing, null, 2) + '\n');
writeFileSync(path.join(root, 'public/v2/plugins.json'), JSON.stringify(buildCatalog(), null, 2) + '\n');
writeFileSync(path.join(output, 'release.json'), JSON.stringify({ tag, asset }, null, 2) + '\n', { flag: 'wx' });
