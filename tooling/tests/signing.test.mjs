import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  readFileSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import test from "node:test";
import { unzipSync } from "fflate";
import { read, sha256, verifySignature } from "../lib/common.mjs";
import { paths } from "../lib/paths.mjs";
import { buildCatalog, validateRecords } from "../lib/records.mjs";
import { candidate, createStore, putExtension } from "./fixtures.mjs";

test(
  "real author pack -> isolated key initialization -> selective signing -> catalog package validation",
  { skip: !process.env.NYATERM_PLUGIN_CLI },
  (t) => {
    const store = createStore(t, true);
    store.put(paths.keys, { version: 1, keys: [] });
    const source = path.join(store.temp, "source");
    mkdirSync(source);
    const manifest = {
      manifestVersion: 1,
      id: "example.tools",
      name: "Tools",
      version: "1.0.0",
      publisher: "example",
      description: "Signing test",
      engine: ">=1.0.0",
      permissions: ["session.read"],
    };
    writeFileSync(path.join(source, "manifest.json"), JSON.stringify(manifest));
    const candidateFile = path.join(store.temp, "candidate.nyap");
    const pack = spawnSync(
      process.env.NYATERM_PLUGIN_CLI,
      ["pack", source, candidateFile, "1.2.12"],
      {
        encoding: "utf8",
        windowsHide: true,
      },
    );
    assert.equal(pack.status, 0, pack.stderr);
    const bytes = readFileSync(candidateFile);
    const submissions = ["universal", "linux-x86_64", "windows-x86_64"].map(
      (target) => ({
        ...candidate("1.0.0", target),
        sha256: sha256(bytes),
        size: bytes.length,
      }),
    );
    const files = putExtension(store, manifest.id, [], submissions);
    const listingBefore = readFileSync(path.join(store.base, files.listing));
    const remainingPath = `${files.submissions}/1.0.0/windows-x86_64.json`;
    const remainingBefore = readFileSync(path.join(store.base, remainingPath));
    const trust = path.join(
      store.temp,
      "platform/src-tauri/crates/nyaterm-plugin-runtime/src/trust.rs",
    );
    mkdirSync(path.dirname(trust), { recursive: true });
    writeFileSync(
      trust,
      "pub const OFFICIAL_PLUGIN_KEYS: &[(&str, &str)] = &[];\n",
    );
    const privateFile = path.join(store.temp, "private.pem");
    const run = (script, args = [], env = {}) =>
      spawnSync(
        process.execPath,
        [path.join(store.base, "tooling/commands", script), ...args],
        {
          cwd: store.temp,
          encoding: "utf8",
          windowsHide: true,
          env: { ...process.env, ...env },
        },
      );
    const initialized = run("init-signing-key.mjs", [
      "test-key",
      privateFile,
      trust,
    ]);
    assert.equal(initialized.status, 0, initialized.stderr);
    const keys = read(paths.keys, store.base);
    assert.ok(readFileSync(trust, "utf8").includes(keys.keys[0].publicKey));
    const output = path.join(store.temp, "signed");
    const signedFile = path.join(output, "example.tools-1.0.0-universal.nyap");
    const mock = path.join(store.temp, "mock-download.mjs");
    writeFileSync(
      mock,
      `import { readFileSync } from "node:fs";
globalThis.fetch = async (url) => {
  const bytes = readFileSync(String(url).includes("nyaterm-plugins/releases") ? process.env.NYATERM_TEST_SIGNED_PACKAGE : process.env.NYATERM_TEST_PACKAGE);
  return new Response(bytes, { status: 200, headers: { "content-length": String(bytes.length) } });
};
`,
    );
    const env = {
      NODE_OPTIONS: `--import=${pathToFileURL(mock).href}`,
      NYATERM_TEST_PACKAGE: candidateFile,
      NYATERM_TEST_SIGNED_PACKAGE: signedFile,
      NYATERM_STORE_SIGNING_KEY: readFileSync(privateFile, "utf8"),
    };
    const sign = (target, version = "1.0.0") =>
      run(
        "sign-plugin.mjs",
        [manifest.id, version, target, "test-key", output],
        env,
      );
    let signed = sign("universal");
    assert.equal(signed.status, 0, signed.stderr);
    const entries = unzipSync(readFileSync(signedFile));
    verifySignature(
      entries["checksums.json"],
      JSON.parse(Buffer.from(entries["signature.json"]).toString()),
      keys.keys[0],
    );
    const releasePath = `${files.releases}/1.0.0.json`;
    const published = read(releasePath, store.base);
    assert.equal(
      published.artifacts[0].sha256,
      sha256(readFileSync(signedFile)),
    );
    assert.equal(
      existsSync(
        path.join(store.base, `${files.submissions}/1.0.0/universal.json`),
      ),
      false,
    );
    assert.equal(validateRecords(store.base).plugins[0].candidates.length, 2);
    assert.deepEqual(
      readFileSync(path.join(store.base, remainingPath)),
      remainingBefore,
    );
    assert.deepEqual(
      readFileSync(path.join(store.base, files.listing)),
      listingBefore,
    );
    assert.deepEqual(
      read(paths.catalog, store.base),
      buildCatalog(validateRecords(store.base).plugins),
    );
    let checked = run("validate.mjs", ["--packages"], env);
    assert.equal(checked.status, 0, checked.stderr);

    const assetBefore = readFileSync(signedFile);
    signed = sign("universal");
    assert.notEqual(signed.status, 0);
    assert.match(signed.stderr, /No approved candidate/);
    assert.deepEqual(readFileSync(signedFile), assetBefore);

    // Refuse to overwrite an existing output asset before consuming its submission.
    const linuxAsset = path.join(
      output,
      "example.tools-1.0.0-linux-x86_64.nyap",
    );
    writeFileSync(linuxAsset, "existing asset");
    signed = sign("linux-x86_64");
    assert.notEqual(signed.status, 0);
    assert.match(signed.stderr, /EEXIST/);
    assert.equal(readFileSync(linuxAsset, "utf8"), "existing asset");
    assert.deepEqual(read(releasePath, store.base), published);
    assert.equal(validateRecords(store.base).plugins[0].candidates.length, 2);
    unlinkSync(linuxAsset);

    signed = sign("linux-x86_64");
    assert.equal(signed.status, 0, signed.stderr);
    const appended = read(releasePath, store.base);
    assert.equal(appended.releasedAt, published.releasedAt);
    assert.equal(appended.releaseNotes, published.releaseNotes);
    assert.deepEqual(
      appended.artifacts.find((artifact) => artifact.target === "universal"),
      published.artifacts[0],
    );
    assert.deepEqual(
      appended.artifacts.map((artifact) => artifact.target),
      ["linux-x86_64", "universal"],
    );
    assert.equal(validateRecords(store.base).plugins[0].candidates.length, 1);
    assert.deepEqual(
      readFileSync(path.join(store.base, remainingPath)),
      remainingBefore,
    );
    assert.deepEqual(readFileSync(signedFile), assetBefore);
    checked = run("validate.mjs", ["--packages"], env);
    assert.equal(checked.status, 0, checked.stderr);

    // Submitted package identity/permissions must still match after restructuring.
    store.put(`${files.submissions}/2.0.0/universal.json`, {
      ...submissions[0],
      version: "2.0.0",
      permissions: ["native"],
    });
    const mismatch = sign("universal", "2.0.0");
    assert.notEqual(mismatch.status, 0);
    assert.match(mismatch.stderr, /manifest differs/);
    assert.equal(
      existsSync(
        path.join(store.base, `${files.submissions}/2.0.0/universal.json`),
      ),
      true,
    );
    assert.deepEqual(read(releasePath, store.base), appended);
  },
);
