import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  cpSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import test from "node:test";
import { unzipSync } from "fflate";
import { root, sha256, verifySignature } from "./common.mjs";

test(
  "real author pack -> isolated key initialization -> signing -> catalog package validation",
  { skip: !process.env.NYATERM_PLUGIN_CLI },
  () => {
    const temp = mkdtempSync(path.join(os.tmpdir(), "nyaterm-signing-test-"));
    assert.ok(
      path.resolve(temp).startsWith(path.resolve(os.tmpdir()) + path.sep),
    );
    try {
      const store = path.join(temp, "store");
      mkdirSync(store);
      for (const directory of ["scripts", "schemas"])
        cpSync(path.join(root, directory), path.join(store, directory), {
          recursive: true,
        });
      for (const directory of ["plugins", "publishers", "catalog"])
        mkdirSync(path.join(store, directory));
      for (const file of ["package.json", "revoked.json"])
        cpSync(path.join(root, file), path.join(store, file));
      symlinkSync(
        path.join(root, "node_modules"),
        path.join(store, "node_modules"),
        process.platform === "win32" ? "junction" : "dir",
      );
      const put = (file, value) =>
        writeFileSync(
          path.join(store, file),
          `${JSON.stringify(value, null, 2)}\n`,
        );
      put("signing-keys.json", { version: 1, keys: [] });
      put("publishers/example.json", {
        id: "example",
        name: "Example",
        status: "unverified",
      });
      const source = path.join(temp, "source");
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
      writeFileSync(
        path.join(source, "manifest.json"),
        JSON.stringify(manifest),
      );
      const candidateFile = path.join(temp, "candidate.nyap");
      const pack = spawnSync(
        process.env.NYATERM_PLUGIN_CLI,
        ["pack", source, candidateFile, "1.2.12"],
        { encoding: "utf8", windowsHide: true },
      );
      assert.equal(pack.status, 0, pack.stderr);
      const bytes = readFileSync(candidateFile);
      const candidate = {
        version: "1.0.0",
        target: "universal",
        url: "https://github.com/example/tools/releases/download/v1.0.0/candidate.nyap",
        sha256: sha256(bytes),
        size: bytes.length,
        permissions: ["session.read"],
        releaseNotes: "Initial release",
      };
      const plugin = {
        id: manifest.id,
        name: manifest.name,
        description: manifest.description,
        publisher: manifest.publisher,
        verified: false,
        tags: [],
        source: "https://github.com/example/tools",
        homepage: "https://github.com/example/tools",
        license: "MIT",
        permissions: candidate.permissions,
        latestVersion: null,
        versions: [],
        candidates: [candidate],
      };
      put("plugins/example.tools.json", plugin);
      const trust = path.join(
        temp,
        "platform/src-tauri/crates/nyaterm-plugin-runtime/src/trust.rs",
      );
      mkdirSync(path.dirname(trust), { recursive: true });
      writeFileSync(
        trust,
        "pub const OFFICIAL_PLUGIN_KEYS: &[(&str, &str)] = &[];\n",
      );
      const privateFile = path.join(temp, "private.pem");
      const run = (script, args = [], env = {}) =>
        spawnSync(
          process.execPath,
          [path.join(store, "scripts", script), ...args],
          {
            cwd: store,
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
      const keys = JSON.parse(
        readFileSync(path.join(store, "signing-keys.json")),
      );
      assert.ok(readFileSync(trust, "utf8").includes(keys.keys[0].publicKey));
      const output = path.join(temp, "signed");
      const signedFile = path.join(
        output,
        "example.tools-1.0.0-universal.nyap",
      );
      const mock = path.join(temp, "mock-download.mjs");
      writeFileSync(
        mock,
        `import { readFileSync } from "node:fs";\nglobalThis.fetch = async (url) => { const bytes = readFileSync(String(url).includes("nyaterm-plugins/releases") ? process.env.NYATERM_TEST_SIGNED_PACKAGE : process.env.NYATERM_TEST_PACKAGE); return new Response(bytes, { status: 200, headers: { "content-length": String(bytes.length) } }); };\n`,
      );
      const env = {
        NODE_OPTIONS: `--import=${pathToFileURL(mock).href}`,
        NYATERM_TEST_PACKAGE: candidateFile,
        NYATERM_TEST_SIGNED_PACKAGE: signedFile,
        NYATERM_STORE_SIGNING_KEY: readFileSync(privateFile, "utf8"),
      };
      const signed = run(
        "sign-plugin.mjs",
        [manifest.id, "1.0.0", "universal", "test-key", output],
        env,
      );
      assert.equal(signed.status, 0, signed.stderr);
      const entries = unzipSync(readFileSync(signedFile));
      verifySignature(
        entries["checksums.json"],
        JSON.parse(Buffer.from(entries["signature.json"]).toString()),
        keys.keys[0],
      );
      const check = run("validate.mjs", ["--packages"], env);
      assert.equal(check.status, 0, check.stderr);
      const published = JSON.parse(
        readFileSync(path.join(store, "plugins/example.tools.json")),
      );
      assert.equal(
        published.versions[0].artifacts[0].sha256,
        sha256(readFileSync(signedFile)),
      );
      assert.equal(published.candidates.length, 0);
      // A changed candidate cannot be signed under reviewed identity/permissions.
      plugin.candidates[0].permissions = ["native"];
      put("plugins/example.tools.json", plugin);
      const mismatch = run(
        "sign-plugin.mjs",
        [manifest.id, "1.0.0", "universal", "test-key", output],
        env,
      );
      assert.notEqual(mismatch.status, 0);
      assert.match(mismatch.stderr, /manifest differs/);
    } finally {
      rmSync(temp, { recursive: true, force: true });
    }
  },
);
