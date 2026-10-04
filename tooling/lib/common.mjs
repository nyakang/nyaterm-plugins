import { createHash, createPublicKey, verify } from "node:crypto";
import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { root, paths } from "./paths.mjs";
export { root } from "./paths.mjs";
import Ajv from "ajv";
import addFormats from "ajv-formats";
import semver from "semver";

export const read = (file, base = root) =>
  JSON.parse(readFileSync(path.join(base, file), "utf8"));
export const write = (file, value, base = root) => {
  const output = path.join(base, file);
  mkdirSync(path.dirname(output), { recursive: true });
  writeFileSync(output, `${JSON.stringify(value, null, 2)}\n`);
};
export const sha256 = (bytes) =>
  createHash("sha256").update(bytes).digest("hex");
const ajv = new Ajv({ allErrors: true, strict: true });
addFormats(ajv);
for (const file of ["candidate", "listing", "release", "catalog"])
  ajv.addSchema(read(`${paths.schemas}/${file}.schema.json`));
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
