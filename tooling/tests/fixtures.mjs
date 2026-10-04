import { cpSync, mkdirSync, mkdtempSync, rmSync, symlinkSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { write } from "../lib/common.mjs";
import { extensionPaths, paths, root } from "../lib/paths.mjs";

export function createStore(t, commands = false) {
  const temp = mkdtempSync(path.join(os.tmpdir(), "nyaterm-store-test-"));
  t.after(() => rmSync(temp, { recursive: true, force: true }));
  const base = path.join(temp, "store");
  mkdirSync(path.join(base, paths.extensions), { recursive: true });
  const put = (file, value) => write(file, value, base);
  put(paths.keys, { version: 1, keys: [testKey] });
  put(paths.revoked, { version: 1, artifacts: [], keys: [] });
  put(`${paths.publishers}/example.json`, {
    id: "example",
    name: "Example",
    status: "verified",
  });
  if (commands) {
    cpSync(path.join(root, "tooling"), path.join(base, "tooling"), {
      recursive: true,
    });
    cpSync(path.join(root, "package.json"), path.join(base, "package.json"));
    symlinkSync(
      path.join(root, "node_modules"),
      path.join(base, "node_modules"),
      process.platform === "win32" ? "junction" : "dir",
    );
  }
  return { temp, base, put };
}

export const testKey = {
  id: "test-key",
  algorithm: "ed25519",
  publicKey: Buffer.alloc(32, 1).toString("base64"),
  status: "active",
  purpose: "repository-package-signing",
};

export const listing = (id = "example.tools") => ({
  id,
  name: "Tools",
  description: "Tools for NyaTerm sessions.",
  publisher: "example",
  tags: ["session"],
  source: "https://github.com/example/tools",
  homepage: "https://github.com/example/tools",
  license: "MIT",
});

export const candidate = (
  version = "1.0.0",
  target = "universal",
  requested = ["session.read"],
) => ({
  version,
  target,
  url: "https://github.com/example/tools/releases/download/v1.0.0/tools.nyap",
  sha256: "a".repeat(64),
  size: 100,
  permissions: requested,
  releaseNotes: "Initial release.",
});

export const release = (
  version = "1.0.0",
  targets = ["universal"],
  requested = ["session.read"],
) => ({
  version,
  releasedAt: "2026-10-03T00:00:00Z",
  releaseNotes: "Released tools.",
  permissions: requested,
  artifacts: targets.map((target) => ({
    target,
    url: `https://github.com/nyakang/nyaterm-plugins/releases/download/plugin-example.tools-v${version}/${target}.nyap`,
    sha256: "b".repeat(64),
    size: 120,
    signingKeyId: testKey.id,
  })),
});

export function putExtension(
  store,
  id = "example.tools",
  releases = [],
  submissions = [],
) {
  const files = extensionPaths(id);
  store.put(files.listing, listing(id));
  for (const value of releases)
    store.put(`${files.releases}/${value.version}.json`, value);
  for (const value of submissions)
    store.put(
      `${files.submissions}/${value.version}/${value.target}.json`,
      value,
    );
  return files;
}
