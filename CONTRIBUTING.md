# Submitting a plugin

Keep source in your own GitHub repository. Use `nyaterm-plugin create`, build your plugin, then `nyaterm-plugin pack` and publish an unsigned `.nyap` as a release asset. Store CI must not build or execute candidate code.

## Publisher and listing

Add `trust/publishers/<publisher-id>.json` and an extension directory through a PR. The publisher ID must match the package's manifest publisher exactly; reviewers maintain its `verified`, `unverified` or `suspended` status.

Create `extensions/example.tools/listing.json` with identity and introduction only:

```json
{
  "id": "example.tools",
  "name": "Tools",
  "description": "Tools for NyaTerm sessions.",
  "publisher": "example",
  "tags": ["session"],
  "source": "https://github.com/example/tools",
  "homepage": "https://github.com/example/tools",
  "license": "MIT"
}
```

Do not add `verified`, `latestVersion`, top-level permissions, candidates or versions to the listing. These are derived or stored separately. The directory name must match the listing ID. Only released extensions appear in the public catalog.

## Submit an unsigned package

Create `extensions/example.tools/submissions/1.0.0/universal.json` (replace URL, hash and size with actual values):

```json
{
  "version": "1.0.0",
  "target": "universal",
  "url": "https://github.com/example/tools/releases/download/v1.0.0/tools.nyap",
  "sha256": "replace-with-64-lowercase-hex-characters",
  "size": 123456,
  "permissions": ["session.read"],
  "releaseNotes": "Initial release."
}
```

The version directory and target filename must match the record. Each target gets its own file. All submitted and published targets for a version must request identical permissions. Use platform-specific targets for native plugins unless all advertised platforms are included in one universal package.

Run `npm run catalog`, `npm run validate` and `npm test`. CI downloads bounded packages and uses the platform inspector to check content checksums and manifests. Reviewers check publisher identity, source/release provenance, license, permissions, native executables and remote scripts.

## Publish an approved submission

Merge the approved submission first. A maintainer dispatches **Sign approved plugin** on `main` with its plugin ID, version, target and active key ID, then approves the protected environment job.

The job signs the exact reviewed package, publishes an immutable GitHub Release asset, writes `extensions/<plugin-id>/releases/<version>.json`, deletes only the selected submission and regenerates `public/v1/plugins.json`. Review and merge the generated PR separately. Authors do not submit signed release records directly.

A release record contains `version`, `releasedAt`, `releaseNotes`, `permissions`, and `artifacts`. Each artifact contains `target`, the official release URL, the final signed archive's `sha256` and `size`, and `signingKeyId`. Additional targets append to that version without changing existing artifacts, its original timestamp or release notes.

Published versions are immutable. Bump the plugin version for content changes. The generated catalog selects the highest released SemVer automatically and excludes unsigned submissions. Store metadata, keys and catalog protocol stay separate from the plugin package manifest.
