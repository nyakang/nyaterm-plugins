import { readdirSync, readFileSync, existsSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import semver from 'semver';

export const root = fileURLToPath(new URL('../../', import.meta.url));
export const capabilities = new Set(['network','local_files_read','local_files_write','remote_files_read','remote_files_write','sessions_read','terminal_read','terminal_write','remote_execute','process_execute','storage','ui']);
export const read = file => JSON.parse(readFileSync(file, 'utf8'));
const assert = (condition, message) => { if (!condition) throw new Error(message); };
const fields = (value, names) => assert(value && typeof value === 'object' && !Array.isArray(value) && Object.keys(value).every(key => names.includes(key)), 'Unknown schema field');
const utf8 = value => Buffer.byteLength(value, 'utf8');
const idPattern = /^[a-z][a-z0-9_-]{0,95}$/;
const validId = id => typeof id === 'string' && idPattern.test(id) && !/^(con|prn|aux|nul|com[1-9]|lpt[1-9])$/.test(id);

export function validateListing(listing) {
  fields(listing, ['id', 'name', 'description', 'publisher', 'releases']);
  assert(listing && validId(listing.id) && !listing.id.startsWith('nyaterm-native-'), 'Invalid or reserved extension ID');
  assert(typeof listing.name === 'string' && listing.name.length > 0 && utf8(listing.name) <= 160, 'Invalid extension name');
  assert(typeof listing.description === 'string' && utf8(listing.description) <= 4096, 'Invalid description');
  assert(typeof listing.publisher === 'string' && validId(listing.publisher), 'Invalid publisher');
  assert(Array.isArray(listing.releases) && listing.releases.length <= 128, 'Invalid releases');
  const versions = new Set();
  for (const release of listing.releases) {
    fields(release, ['version', 'host_version', 'api_version', 'capabilities', 'artifact']);
    assert(semver.valid(release.version) && !versions.has(release.version), 'Invalid or duplicate version');
    versions.add(release.version);
    assert(typeof release.host_version === 'string' && semver.validRange(release.host_version.replaceAll(',', ' ')), 'Invalid host version requirement');
    assert(release.api_version === '2.0.0', 'Unsupported extension API');
    assert(Array.isArray(release.capabilities) && release.capabilities.every(c => capabilities.has(c)) && new Set(release.capabilities).size === release.capabilities.length, 'Invalid capabilities');
    const artifact = release.artifact;
    fields(artifact, ['url', 'sha256', 'size', 'signature']);
    const url = new URL(artifact.url);
    assert(url.protocol === 'https:' && !url.username && !url.password && !url.hash && artifact.url.length <= 4096, 'Invalid artifact URL');
    assert(/^[a-f0-9]{64}$/.test(artifact.sha256), 'Invalid artifact digest');
    assert(Number.isSafeInteger(artifact.size) && artifact.size > 0 && artifact.size <= 64 * 1024 * 1024, 'Invalid artifact size');
    assert(typeof artifact.signature === 'string' && artifact.signature.length > 0 && artifact.signature.length <= 8192, 'Missing Minisign signature');
  }
  return listing;
}

export function buildCatalog(base = root) {
  const directory = path.join(base, 'extensions-v2');
  const revoked = read(path.join(base, 'trust/revoked.json'));
  assert(Array.isArray(revoked.artifacts), 'Invalid artifact revocation list');
  const listings = existsSync(directory) ? readdirSync(directory, { withFileTypes: true }).filter(e => e.isDirectory()).map(entry => {
    const listing = validateListing(read(path.join(directory, entry.name, 'listing.json')));
    assert(listing.id === entry.name, 'Directory and extension ID differ');
    const publisher = read(path.join(base, 'trust/publishers', `${listing.publisher}.json`));
    assert(publisher.id === listing.publisher && ['verified', 'unverified'].includes(publisher.status), 'Publisher is suspended or invalid');
    assert(listing.releases.every(release => !revoked.artifacts.includes(release.artifact.sha256)), 'Catalog contains a revoked artifact');
    return { ...listing, releases: [...listing.releases].sort((a,b) => semver.rcompare(a.version,b.version)) };
  }) : [];
  const catalog = { catalog_version: 2, extensions: listings.sort((a,b) => a.id.localeCompare(b.id, 'en')) };
  assert(listings.length <= 4096 && utf8(JSON.stringify(catalog)) <= 4 * 1024 * 1024, 'Catalog exceeds client bounds');
  return catalog;
}
