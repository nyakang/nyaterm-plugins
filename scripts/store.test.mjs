import assert from "node:assert/strict";
import { generateKeyPairSync, sign } from "node:crypto";
import test from "node:test";
import {
  buildCatalog,
  https,
  matchManifest,
  permissions,
  samePermissions,
  schema,
  validateRecords,
  verifySignature,
} from "./common.mjs";

test("repository starts with a deterministic, schema-valid empty catalog", () => {
  const { plugins } = validateRecords();
  const catalog = buildCatalog(plugins);
  schema("catalog", catalog);
  assert.equal(catalog.repository.id, "nyaterm-official");
  assert.deepEqual(buildCatalog(plugins), catalog);
});
test("URL and permission validation reject credential URLs and wider origins", () => {
  assert.throws(() => https("http://example.com"));
  assert.throws(() => https("https://user:password@example.com"));
  assert.throws(() => permissions(["network:https://example.com/path"]));
  assert.throws(() => permissions(["native", "native"]));
  permissions(["network:https://example.com", "native"]);
});
test("publisher, identity, version and permission mismatches fail", () => {
  const manifest = {
    id: "example.tools",
    publisher: "example",
    version: "1.0.0",
    permissions: ["session.read"],
  };
  const plugin = { id: manifest.id, publisher: manifest.publisher };
  const candidate = {
    version: manifest.version,
    permissions: manifest.permissions,
  };
  matchManifest(manifest, plugin, candidate);
  for (const change of [
    { id: "example.other" },
    { publisher: "attacker" },
    { version: "2.0.0" },
    { permissions: ["session.read", "native"] },
  ])
    assert.throws(() =>
      matchManifest({ ...manifest, ...change }, plugin, candidate),
    );
  assert.ok(samePermissions(["native", "storage"], ["storage", "native"]));
});
test("repository signature covers exact checksum bytes and trusted key identity", () => {
  const { publicKey, privateKey } = generateKeyPairSync("ed25519");
  const key = {
    id: "test",
    publicKey: publicKey
      .export({ type: "spki", format: "der" })
      .subarray(-32)
      .toString("base64"),
  };
  const checksums = Buffer.from('{"manifest.json":"digest"}\n');
  const signature = {
    algorithm: "ed25519",
    keyId: key.id,
    signature: sign(null, checksums, privateKey).toString("base64"),
  };
  verifySignature(checksums, signature, key);
  assert.throws(() =>
    verifySignature(Buffer.from(checksums.toString().trim()), signature, key),
  );
  assert.throws(() =>
    verifySignature(checksums, { ...signature, keyId: "unknown" }, key),
  );
});
