import { extensionPaths, paths, root } from "../lib/paths.mjs";
import { buildCatalog, validateRecords } from "../lib/records.mjs";
import { createPrivateKey, createPublicKey, sign } from "node:crypto";
import { mkdirSync, unlinkSync, writeFileSync } from "node:fs";
import path from "node:path";
import { unzipSync, zipSync } from "fflate";
import {
  download,
  inspect,
  matchManifest,
  samePermissions,
  sha256,
  verifySignature,
  write,
} from "../lib/common.mjs";

const [id, version, target, keyId, outputDirectory] = process.argv.slice(2);
if (!outputDirectory)
  throw new Error(
    "Usage: sign-plugin.mjs <id> <version> <target> <key-id> <output-directory>",
  );
const { plugins, keys, revoked } = validateRecords();
const plugin = plugins.find((p) => p.id === id);
const candidate = plugin?.candidates?.find(
  (c) => c.version === version && c.target === target,
);
if (!candidate)
  throw new Error("No approved candidate in the checked-out Store metadata");
const key = keys.get(keyId);
if (!key || key.status !== "active" || revoked.keys.includes(keyId))
  throw new Error("Signing key is not active");
const privateKey = createPrivateKey(
  process.env.NYATERM_STORE_SIGNING_KEY ?? "",
);
const rawPublic = createPublicKey(privateKey)
  .export({ format: "der", type: "spki" })
  .subarray(-32)
  .toString("base64");
if (privateKey.asymmetricKeyType !== "ed25519" || rawPublic !== key.publicKey)
  throw new Error("Signing secret does not match the reviewed public key");
mkdirSync(outputDirectory, { recursive: true });
const bytes = await download(candidate);
const unsigned = path.join(outputDirectory, "candidate.nyap");
writeFileSync(unsigned, bytes);
const preview = inspect(unsigned);
matchManifest(preview.manifest, plugin, candidate);
if (preview.signature.status !== "unsigned")
  throw new Error("Candidate already has a signature");
const entries = unzipSync(bytes);
const signature = {
  algorithm: "ed25519",
  keyId,
  signature: sign(null, entries["checksums.json"], privateKey).toString(
    "base64",
  ),
};
verifySignature(entries["checksums.json"], signature, key);
entries["signature.json"] = Buffer.from(
  `${JSON.stringify(signature, null, 2)}\n`,
);
const signed = Buffer.from(
  zipSync(entries, { level: 6, mtime: new Date(1980, 0, 1) }),
);
if (signed.length > 128 * 1024 * 1024)
  throw new Error("Signed package exceeds size limit");
const asset = `${id}-${version}-${target}.nyap`;
const signedPath = path.join(outputDirectory, asset);
// Never replace a previously assembled release asset, even on a local retry.
writeFileSync(signedPath, signed, { flag: "wx" });
matchManifest(inspect(signedPath).manifest, plugin, candidate);
const tag = `plugin-${id}-v${version}`;
const artifact = {
  target,
  url: `https://github.com/nyakang/nyaterm-plugins/releases/download/${tag}/${asset}`,
  sha256: sha256(signed),
  size: signed.length,
  signingKeyId: keyId,
};
let release = plugin.versions.find((v) => v.version === version);
if (release && !samePermissions(release.permissions, candidate.permissions))
  throw new Error("Targets of a version must request the same permissions");
if (!release) {
  release = {
    version,
    releasedAt: new Date().toISOString(),
    releaseNotes: candidate.releaseNotes,
    permissions: candidate.permissions,
    artifacts: [],
  };
}
release.artifacts.push(artifact);
release.artifacts.sort((a, b) =>
  a.target < b.target ? -1 : a.target > b.target ? 1 : 0,
);
const files = extensionPaths(id);
write(`${files.releases}/${version}.json`, release);
unlinkSync(path.join(root, `${files.submissions}/${version}/${target}.json`));
write(paths.catalog, buildCatalog(validateRecords().plugins));
writeFileSync(
  path.join(outputDirectory, "release.json"),
  `${JSON.stringify({ tag, asset, signedPath, sha256: artifact.sha256 }, null, 2)}\n`,
);
process.stdout.write(
  `Signed ${id} ${version} ${target}; final SHA-256: ${artifact.sha256}\n`,
);
