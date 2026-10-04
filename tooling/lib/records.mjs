import { readdirSync } from "node:fs";
import path from "node:path";
import semver from "semver";
import {
  https,
  permissions,
  read,
  samePermissions,
  schema,
  version,
} from "./common.mjs";
import { extensionPaths, paths, root } from "./paths.mjs";

const compare = (a, b) => (a < b ? -1 : a > b ? 1 : 0);
const validId = (id) =>
  typeof id === "string" &&
  /^[a-z0-9][a-z0-9._-]{0,127}$/.test(id) &&
  !id.includes("..");

function entries(directory, base, optional = false) {
  try {
    return readdirSync(path.join(base, directory), { withFileTypes: true })
      .filter((entry) => entry.name !== ".gitkeep")
      .sort((a, b) => compare(a.name, b.name));
  } catch (error) {
    if (optional && error.code === "ENOENT") return [];
    throw error;
  }
}

function jsonRecords(directory, base, optional = false) {
  return entries(directory, base, optional).map((entry) => {
    if (!entry.isFile() || !entry.name.endsWith(".json"))
      throw new Error(`Expected JSON file: ${directory}/${entry.name}`);
    return {
      file: entry.name,
      value: read(`${directory}/${entry.name}`, base),
    };
  });
}

export function validateRecords(base = root) {
  const publishers = new Map(
    jsonRecords(paths.publishers, base).map(({ file, value }) => {
      if (
        !validId(value.id) ||
        file !== `${value.id}.json` ||
        !value.name ||
        !["verified", "unverified", "suspended"].includes(value.status)
      )
        throw new Error(`Invalid publisher: ${file}`);
      return [value.id, value];
    }),
  );
  const document = read(paths.keys, base);
  if (document.version !== 1 || !Array.isArray(document.keys))
    throw new Error("Invalid signing keys document");
  const keys = new Map();
  for (const key of document.keys) {
    if (
      keys.has(key.id) ||
      !validId(key.id) ||
      key.algorithm !== "ed25519" ||
      key.purpose !== "repository-package-signing" ||
      !["active", "retired", "revoked"].includes(key.status) ||
      !/^[A-Za-z0-9+/]{43}=$/.test(key.publicKey) ||
      Buffer.from(key.publicKey, "base64").length !== 32
    )
      throw new Error("Invalid signing key");
    keys.set(key.id, key);
  }
  const revoked = read(paths.revoked, base);
  if (
    revoked.version !== 1 ||
    !Array.isArray(revoked.artifacts) ||
    !Array.isArray(revoked.keys) ||
    revoked.artifacts.some((hash) => !/^[a-f0-9]{64}$/.test(hash)) ||
    revoked.keys.some((id) => !keys.has(id))
  )
    throw new Error("Invalid revocations");

  const plugins = entries(paths.extensions, base).map((entry) => {
    if (
      !entry.isDirectory() ||
      !validId(entry.name) ||
      !entry.name.includes(".")
    )
      throw new Error(`Invalid extension directory: ${entry.name}`);
    const files = extensionPaths(entry.name);
    const listing = read(files.listing, base);
    schema("listing", listing);
    if (
      listing.id !== entry.name ||
      !validId(listing.id) ||
      !listing.name.trim()
    )
      throw new Error(
        `Listing identity disagrees with directory: ${entry.name}`,
      );
    const publisher = publishers.get(listing.publisher);
    if (!publisher || publisher.status === "suspended")
      throw new Error(`Unknown or suspended publisher: ${listing.publisher}`);
    https(listing.source);
    https(listing.homepage);

    const versions = jsonRecords(files.releases, base, true)
      .map(({ file, value: release }) => {
        schema("release", release);
        version(release.version);
        if (file !== `${release.version}.json`)
          throw new Error(`Release version disagrees with filename: ${file}`);
        permissions(release.permissions);
        const targets = new Set();
        for (const artifact of release.artifacts) {
          https(artifact.url);
          if (targets.has(artifact.target))
            throw new Error("Duplicate artifact target");
          targets.add(artifact.target);
          const key = keys.get(artifact.signingKeyId);
          if (
            !key ||
            key.status === "revoked" ||
            revoked.keys.includes(key.id) ||
            revoked.artifacts.includes(artifact.sha256)
          )
            throw new Error("Untrusted or revoked catalog artifact");
        }
        release.artifacts.sort((a, b) => compare(a.target, b.target));
        return release;
      })
      .sort(
        (a, b) =>
          semver.rcompare(a.version, b.version) ||
          compare(a.version, b.version),
      );
    if (versions.length > 128) throw new Error("Too many released versions");

    const candidates = entries(files.submissions, base, true).flatMap(
      (directory) => {
        if (!directory.isDirectory())
          throw new Error(
            `Expected submission version directory: ${directory.name}`,
          );
        version(directory.name);
        return jsonRecords(`${files.submissions}/${directory.name}`, base).map(
          ({ file, value: candidate }) => {
            schema("candidate", candidate);
            version(candidate.version);
            if (
              candidate.version !== directory.name ||
              file !== `${candidate.target}.json`
            )
              throw new Error(
                `Submission version or target disagrees with path: ${directory.name}/${file}`,
              );
            permissions(candidate.permissions);
            https(candidate.url);
            return candidate;
          },
        );
      },
    );
    if (candidates.length > 128) throw new Error("Too many candidates");
    const seen = new Set();
    const versionPermissions = new Map(
      versions.map((release) => [release.version, release.permissions]),
    );
    for (const candidate of candidates) {
      const id = `${candidate.version}/${candidate.target}`;
      if (
        seen.has(id) ||
        versions.some(
          (release) =>
            release.version === candidate.version &&
            release.artifacts.some(
              (artifact) => artifact.target === candidate.target,
            ),
        )
      )
        throw new Error("Duplicate/released candidate");
      seen.add(id);
      const requested = versionPermissions.get(candidate.version);
      if (requested && !samePermissions(requested, candidate.permissions))
        throw new Error(
          "Targets of a version must request the same permissions",
        );
      versionPermissions.set(candidate.version, candidate.permissions);
    }
    const latest = versions[0];
    return {
      ...listing,
      verified: publisher.status === "verified",
      permissions: latest?.permissions ?? [],
      latestVersion: latest?.version ?? null,
      versions,
      candidates,
    };
  });
  if (plugins.length > 2048) throw new Error("Too many extensions");
  return { plugins, keys, revoked };
}

export function buildCatalog(plugins) {
  const published = plugins
    .filter((plugin) => plugin.versions.length)
    .map(({ candidates, ...plugin }) => ({
      ...plugin,
      versions: plugin.versions
        .map((release) => ({
          ...release,
          artifacts: [...release.artifacts].sort((a, b) =>
            compare(a.target, b.target),
          ),
        }))
        .sort(
          (a, b) =>
            semver.rcompare(a.version, b.version) ||
            compare(a.version, b.version),
        ),
    }))
    .sort((a, b) => compare(a.id, b.id));
  const dates = published
    .flatMap((plugin) => plugin.versions.map((release) => release.releasedAt))
    .sort();
  const catalog = {
    catalogVersion: 1,
    repository: { id: "nyaterm-official", name: "NyaTerm Plugin Store" },
    generatedAt: dates.at(-1) ?? "2026-10-03T00:00:00Z",
    plugins: published,
  };
  schema("catalog", catalog);
  if (
    Buffer.byteLength(`${JSON.stringify(catalog, null, 2)}\n`) >
    4 * 1024 * 1024
  )
    throw new Error("Store catalog is too large");
  return catalog;
}
