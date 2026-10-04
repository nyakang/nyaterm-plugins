import assert from "node:assert/strict";
import { readFileSync, renameSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import path from "node:path";
import test from "node:test";
import { read } from "../lib/common.mjs";
import { paths } from "../lib/paths.mjs";
import { buildCatalog, validateRecords } from "../lib/records.mjs";
import {
  candidate,
  createStore,
  listing,
  putExtension,
  release,
  testKey,
} from "./fixtures.mjs";

test("candidate-only listings stay private and verified status comes from publisher records", (t) => {
  const store = createStore(t);
  putExtension(store, "example.tools", [], [candidate()]);
  const { plugins } = validateRecords(store.base);
  assert.equal(plugins[0].verified, true);
  assert.equal(plugins[0].latestVersion, null);
  assert.deepEqual(plugins[0].permissions, []);
  assert.deepEqual(buildCatalog(plugins).plugins, []);
  store.put(`${paths.publishers}/example.json`, {
    id: "example",
    name: "Example",
    status: "unverified",
  });
  assert.equal(validateRecords(store.base).plugins[0].verified, false);
});

test("catalog aggregates versions and targets deterministically without modifying source records", (t) => {
  const store = createStore(t);
  const files = putExtension(
    store,
    "example.tools",
    [
      release("1.0.0", ["windows-x86_64", "universal"]),
      release(
        "2.0.0",
        ["macos-aarch64", "linux-x86_64"],
        ["native", "storage"],
      ),
    ],
    [candidate("3.0.0")],
  );
  putExtension(store, "example.aaa", [release()]);
  const before = readFileSync(
    path.join(store.base, `${files.releases}/1.0.0.json`),
  );
  const { plugins } = validateRecords(store.base);
  const catalog = buildCatalog(plugins);
  assert.deepEqual(
    catalog.plugins.map((plugin) => plugin.id),
    ["example.aaa", "example.tools"],
  );
  const plugin = catalog.plugins[1];
  assert.equal(plugin.latestVersion, "2.0.0");
  assert.deepEqual(plugin.permissions, ["native", "storage"]);
  assert.deepEqual(
    plugin.versions.map((value) => value.version),
    ["2.0.0", "1.0.0"],
  );
  assert.deepEqual(
    plugin.versions[1].artifacts.map((value) => value.target),
    ["universal", "windows-x86_64"],
  );
  assert.equal("candidates" in plugin, false);
  assert.deepEqual(buildCatalog([...plugins].reverse()), catalog);
  assert.deepEqual(buildCatalog(validateRecords(store.base).plugins), catalog);
  assert.deepEqual(
    readFileSync(path.join(store.base, `${files.releases}/1.0.0.json`)),
    before,
  );
  // Optional artifact for the host's Rust Catalog::parse interoperability check.
  if (process.env.NYATERM_TEST_CATALOG_OUTPUT)
    writeFileSync(
      process.env.NYATERM_TEST_CATALOG_OUTPUT,
      `${JSON.stringify(catalog, null, 2)}\n`,
    );
});

test("listing ID, release filename and submission version/target must match their paths", (t) => {
  const store = createStore(t);
  const files = putExtension(
    store,
    "example.tools",
    [release()],
    [candidate("2.0.0")],
  );
  store.put(files.listing, listing("example.other"));
  assert.throws(() => validateRecords(store.base), /identity disagrees/);
  store.put(files.listing, listing());
  const original = path.join(store.base, `${files.releases}/1.0.0.json`);
  const wrong = path.join(store.base, `${files.releases}/9.0.0.json`);
  renameSync(original, wrong);
  assert.throws(() => validateRecords(store.base), /version disagrees/);
  renameSync(wrong, original);
  store.put(`${files.submissions}/2.0.0/universal.json`, candidate("3.0.0"));
  assert.throws(
    () => validateRecords(store.base),
    /version or target disagrees/,
  );
  store.put(
    `${files.submissions}/2.0.0/universal.json`,
    candidate("2.0.0", "linux-x86_64"),
  );
  assert.throws(
    () => validateRecords(store.base),
    /version or target disagrees/,
  );
});

test("listing cannot override generated verification, permissions or versions", (t) => {
  const store = createStore(t);
  const files = putExtension(store);
  for (const change of [
    { verified: true },
    { permissions: ["native"] },
    { latestVersion: "1.0.0" },
    { versions: [] },
  ]) {
    store.put(files.listing, { ...listing(), ...change });
    assert.throws(() => validateRecords(store.base), /additional properties/);
  }
});

test("duplicate published targets and already released candidates are rejected", (t) => {
  const store = createStore(t);
  const files = putExtension(store, "example.tools", [
    release("1.0.0", ["universal", "universal"]),
  ]);
  assert.throws(
    () => validateRecords(store.base),
    /duplicate items|Duplicate artifact target/,
  );
  const duplicate = release("1.0.0", ["universal", "universal"]);
  duplicate.artifacts[1].sha256 = "c".repeat(64);
  store.put(`${files.releases}/1.0.0.json`, duplicate);
  assert.throws(() => validateRecords(store.base), /Duplicate artifact target/);
  store.put(`${files.releases}/1.0.0.json`, release());
  store.put(`${files.submissions}/1.0.0/universal.json`, candidate());
  assert.throws(
    () => validateRecords(store.base),
    /Duplicate\/released candidate/,
  );
});

test("all submitted and published targets of a version must request the same permissions", (t) => {
  const store = createStore(t);
  const files = putExtension(
    store,
    "example.tools",
    [],
    [candidate(), candidate("1.0.0", "linux-x86_64", ["native"])],
  );
  assert.throws(() => validateRecords(store.base), /same permissions/);
  store.put(
    `${files.submissions}/1.0.0/linux-x86_64.json`,
    candidate("1.0.0", "linux-x86_64"),
  );
  store.put(
    `${files.releases}/1.0.0.json`,
    release("1.0.0", ["windows-x86_64"], ["native"]),
  );
  assert.throws(() => validateRecords(store.base), /same permissions/);
});

test("unknown and suspended publishers cannot publish listings", (t) => {
  const store = createStore(t);
  const files = putExtension(store);
  store.put(files.listing, { ...listing(), publisher: "unknown" });
  assert.throws(
    () => validateRecords(store.base),
    /Unknown or suspended publisher/,
  );
  store.put(files.listing, listing());
  store.put(`${paths.publishers}/example.json`, {
    id: "example",
    name: "Example",
    status: "suspended",
  });
  assert.throws(
    () => validateRecords(store.base),
    /Unknown or suspended publisher/,
  );
});

test("unknown signing keys, revoked keys and revoked artifact hashes cannot enter the feed", (t) => {
  const store = createStore(t);
  const files = putExtension(store, "example.tools", [release()]);
  const value = release();
  value.artifacts[0].signingKeyId = "unknown";
  store.put(`${files.releases}/1.0.0.json`, value);
  assert.throws(() => validateRecords(store.base), /Untrusted or revoked/);
  store.put(`${files.releases}/1.0.0.json`, release());
  store.put(paths.keys, {
    version: 1,
    keys: [{ ...testKey, status: "revoked" }],
  });
  assert.throws(() => validateRecords(store.base), /Untrusted or revoked/);
  store.put(paths.keys, { version: 1, keys: [testKey] });
  for (const revoked of [
    { artifacts: [], keys: [testKey.id] },
    { artifacts: ["b".repeat(64)], keys: [] },
  ]) {
    store.put(paths.revoked, { version: 1, ...revoked });
    assert.throws(() => validateRecords(store.base), /Untrusted or revoked/);
  }
});

test("generation and validation commands work outside the repository cwd and detect stale output", (t) => {
  const store = createStore(t, true);
  const files = putExtension(store, "example.tools", [release()]);
  const run = (command) =>
    spawnSync(
      process.execPath,
      [path.join(store.base, "tooling/commands", `${command}.mjs`)],
      {
        cwd: store.temp,
        encoding: "utf8",
        windowsHide: true,
      },
    );
  let result = run("generate-catalog");
  assert.equal(result.status, 0, result.stderr);
  const initial = readFileSync(path.join(store.base, paths.catalog));
  result = run("generate-catalog");
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(readFileSync(path.join(store.base, paths.catalog)), initial);
  result = run("validate");
  assert.equal(result.status, 0, result.stderr);
  store.put(files.listing, {
    ...listing(),
    description: "Changed description",
  });
  result = run("validate");
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /Catalog is stale/);
  assert.equal(
    read(paths.catalog, store.base).plugins[0].description,
    listing().description,
  );
});
