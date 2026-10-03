import { createHash, createPublicKey, verify } from "node:crypto";
import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import Ajv from "ajv";
import addFormats from "ajv-formats";
import semver from "semver";

export const root = fileURLToPath(new URL("../", import.meta.url));
export const read = (file) =>
  JSON.parse(readFileSync(path.join(root, file), "utf8"));
export const write = (file, value) => {
  const output = path.join(root, file);
  mkdirSync(path.dirname(output), { recursive: true });
  writeFileSync(output, `${JSON.stringify(value, null, 2)}\n`);
};
export const sha256 = (bytes) =>
  createHash("sha256").update(bytes).digest("hex");
export const records = (directory) =>
  readdirSync(path.join(root, directory))
    .filter((p) => p.endsWith(".json"))
    .sort()
    .map((file) => ({ file, value: read(`${directory}/${file}`) }));
const ajv = new Ajv({ allErrors: true, strict: true });
addFormats(ajv);
for (const file of ["candidate", "plugin", "catalog"])
  ajv.addSchema(read(`schemas/${file}.schema.json`));
export function schema(name, value) {
  const check = ajv.getSchema(
    `https://nyaterm.app/schemas/store/${name}.schema.json`,
  );
  if (!check(value))
    throw new Error(`${name}: ${ajv.errorsText(check.errors)}`);
}
export function https(value) {
  const url = new URL(value);
  if (
    value.length > 4096 ||
    url.protocol !== "https:" ||
    url.username ||
    url.password ||
    url.hash
  )
    throw new Error("Expected HTTPS URL without credentials or fragment");
  return url;
}
export function permissions(values) {
  if (values.length > 24 || new Set(values).size !== values.length)
    throw new Error("Invalid permissions");
  for (const p of values) {
    if (
      [
        "session.read",
        "terminal.read",
        "terminal.execute",
        "filesystem.read",
        "storage",
        "native",
        "remote.probe",
      ].includes(p)
    )
      continue;
    if (p.startsWith("network:")) {
      const url = https(p.slice(8));
      if (url.origin === p.slice(8)) continue;
    }
    throw new Error(`Invalid permission: ${p}`);
  }
}
export function version(value) {
  if (semver.valid(value) !== value)
    throw new Error(`Invalid canonical version: ${value}`);
}
export function samePermissions(a, b) {
  return JSON.stringify([...a].sort()) === JSON.stringify([...b].sort());
}

export function validateRecords() {
  const publishers = new Map(
    records("publishers").map(({ file, value }) => {
      if (
        file !== `${value.id}.json` ||
        !/^[a-z0-9][a-z0-9._-]{0,127}$/.test(value.id) ||
        value.id.includes("..") ||
        !value.name ||
        !["verified", "unverified", "suspended"].includes(value.status)
      )
        throw new Error(`Invalid publisher: ${file}`);
      return [value.id, value];
    }),
  );
  const document = read("signing-keys.json");
  if (document.version !== 1 || !Array.isArray(document.keys))
    throw new Error("Invalid signing keys document");
  const keys = new Map();
  for (const key of document.keys) {
    if (
      keys.has(key.id) ||
      !/^[a-z0-9][a-z0-9._-]{0,127}$/.test(key.id) ||
      key.id.includes("..") ||
      key.algorithm !== "ed25519" ||
      key.purpose !== "repository-package-signing" ||
      !["active", "retired", "revoked"].includes(key.status) ||
      !/^[A-Za-z0-9+/]{43}=$/.test(key.publicKey) ||
      Buffer.from(key.publicKey, "base64").length !== 32
    )
      throw new Error("Invalid signing key");
    keys.set(key.id, key);
  }
  const revoked = read("revoked.json");
  if (
    revoked.version !== 1 ||
    !Array.isArray(revoked.artifacts) ||
    !Array.isArray(revoked.keys) ||
    revoked.artifacts.some((h) => !/^[a-f0-9]{64}$/.test(h)) ||
    revoked.keys.some((id) => !keys.has(id))
  )
    throw new Error("Invalid revocations");
  const plugins = records("plugins").map(({ file, value: plugin }) => {
    schema("plugin", plugin);
    if (file !== `${plugin.id}.json` || plugin.id.includes(".."))
      throw new Error(`Invalid plugin filename: ${file}`);
    const publisher = publishers.get(plugin.publisher);
    if (
      !publisher ||
      publisher.status === "suspended" ||
      plugin.verified !== (publisher.status === "verified")
    )
      throw new Error(`Publisher record disagrees: ${file}`);
    https(plugin.source);
    https(plugin.homepage);
    permissions(plugin.permissions);
    const seen = new Set();
    for (const v of plugin.versions) {
      version(v.version);
      permissions(v.permissions);
      if (seen.has(v.version)) throw new Error("Duplicate version");
      seen.add(v.version);
      const targets = new Set();
      for (const a of v.artifacts) {
        https(a.url);
        if (targets.has(a.target)) throw new Error("Duplicate artifact target");
        targets.add(a.target);
        const key = keys.get(a.signingKeyId);
        if (
          !key ||
          key.status === "revoked" ||
          revoked.keys.includes(key.id) ||
          revoked.artifacts.includes(a.sha256)
        )
          throw new Error("Untrusted or revoked catalog artifact");
      }
    }
    if (plugin.versions.length) {
      const latest = plugin.versions.find(
        (v) => v.version === plugin.latestVersion,
      );
      if (!latest || !samePermissions(latest.permissions, plugin.permissions))
        throw new Error("Latest version permissions disagree");
    } else if (plugin.latestVersion !== null)
      throw new Error("Unpublished plugin must have null latestVersion");
    const candidates = new Set();
    for (const c of plugin.candidates ?? []) {
      version(c.version);
      permissions(c.permissions);
      https(c.url);
      const id = `${c.version}/${c.target}`;
      if (
        candidates.has(id) ||
        plugin.versions.some(
          (v) =>
            v.version === c.version &&
            v.artifacts.some((a) => a.target === c.target),
        )
      )
        throw new Error("Duplicate/released candidate");
      candidates.add(id);
    }
    return plugin;
  });
  return { plugins, keys, revoked };
}

export function buildCatalog(plugins) {
  // Derive time from metadata so catalog generation is deterministic and CI can check drift.
  const published = plugins
    .filter((p) => p.versions.length)
    .map(({ candidates, ...p }) => p);
  const dates = published
    .flatMap((p) => p.versions.map((v) => v.releasedAt))
    .sort();
  const catalog = {
    catalogVersion: 1,
    repository: { id: "nyaterm-official", name: "NyaTerm Plugin Store" },
    generatedAt: dates.at(-1) ?? "2026-10-03T00:00:00Z",
    plugins: published,
  };
  schema("catalog", catalog);
  return catalog;
}

export function inspect(file) {
  const binary = process.env.NYATERM_PLUGIN_CLI;
  if (!binary)
    throw new Error(
      "Set NYATERM_PLUGIN_CLI to the trusted nyaterm-plugin binary",
    );
  const result = spawnSync(
    binary,
    ["inspect", file, process.env.NYATERM_APP_VERSION ?? "1.2.12"],
    { encoding: "utf8", windowsHide: true, maxBuffer: 2 * 1024 * 1024 },
  );
  if (result.error || result.status !== 0)
    throw new Error(result.error?.message ?? result.stderr);
  return JSON.parse(result.stdout);
}

export function matchManifest(manifest, plugin, release) {
  if (
    manifest.id !== plugin.id ||
    manifest.publisher !== plugin.publisher ||
    manifest.version !== release.version ||
    !samePermissions(manifest.permissions, release.permissions)
  )
    throw new Error("Package manifest differs from reviewed metadata");
  if (
    release.target &&
    manifest.backend &&
    release.target !== "universal" &&
    !manifest.backend.executables[release.target]
  )
    throw new Error("Missing backend for candidate target");
}

export async function download(artifact) {
  let url = https(artifact.url);
  const deadline = AbortSignal.timeout(120_000);
  let response;
  for (let redirects = 0; redirects <= 5; redirects++) {
    response = await fetch(url, { redirect: "manual", signal: deadline });
    if ([301, 302, 303, 307, 308].includes(response.status)) {
      await response.body?.cancel();
      url = https(new URL(response.headers.get("location"), url).href);
      continue;
    }
    break;
  }
  if (!response?.ok || !response.body)
    throw new Error(`Download failed: ${response?.status}`);
  const length = response.headers.get("content-length");
  if (length && Number(length) !== artifact.size) {
    await response.body.cancel();
    throw new Error("Download size mismatch");
  }
  const chunks = [];
  let size = 0;
  for await (const chunk of response.body) {
    size += chunk.length;
    if (size > artifact.size || size > 128 * 1024 * 1024)
      throw new Error("Download exceeds size limit");
    chunks.push(chunk);
  }
  const bytes = Buffer.concat(chunks);
  if (size !== artifact.size || sha256(bytes) !== artifact.sha256)
    throw new Error("Download size or SHA-256 mismatch");
  return bytes;
}

export function publicKey(base64) {
  return createPublicKey({
    key: Buffer.concat([
      Buffer.from("302a300506032b6570032100", "hex"),
      Buffer.from(base64, "base64"),
    ]),
    format: "der",
    type: "spki",
  });
}
export function verifySignature(checksums, signature, key) {
  if (
    signature.algorithm !== "ed25519" ||
    signature.keyId !== key.id ||
    !/^[A-Za-z0-9+/]{86}==$/.test(signature.signature) ||
    !verify(
      null,
      checksums,
      publicKey(key.publicKey),
      Buffer.from(signature.signature, "base64"),
    )
  )
    throw new Error("Invalid repository signature");
}
