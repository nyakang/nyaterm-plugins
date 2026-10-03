# Submitting a plugin

Keep source in your own GitHub repository. Use `nyaterm-plugin create`, build your plugin, then `nyaterm-plugin pack` and publish an **unsigned** `.nyap` as a release asset. Do not build or execute candidate code in Store CI.

Add your publisher record and one `plugins/<publisher.plugin>.json` record through a PR. Publisher IDs must match the package's manifest publisher exactly; verified status is maintained by Store reviewers.

A candidate-only record looks like this (replace URL, hash, size and identity with real values):

```json
{
  "id": "example.tools",
  "name": "Tools",
  "description": "Tools for NyaTerm sessions.",
  "publisher": "example",
  "verified": false,
  "tags": ["session"],
  "source": "https://github.com/example/tools",
  "homepage": "https://github.com/example/tools",
  "license": "MIT",
  "permissions": ["session.read"],
  "latestVersion": null,
  "versions": [],
  "candidates": [
    {
      "version": "1.0.0",
      "target": "universal",
      "url": "https://github.com/example/tools/releases/download/v1.0.0/tools.nyap",
      "sha256": "replace-with-64-lowercase-hex-characters",
      "size": 123456,
      "permissions": ["session.read"],
      "releaseNotes": "Initial release."
    }
  ]
}
```

Run `npm run catalog` and `npm run validate`. CI downloads bounded packages and uses the platform inspector to check complete content checksums and manifests. Reviewers check publisher identity, source/release provenance, license, permission changes, native executables and remote scripts. Merge the approved candidate metadata first; a maintainer then dispatches **Sign approved plugin** on `main` and approves the protected environment job. Review and merge the generated signed catalog PR separately.

Published versions are immutable. Bump the plugin version for changes. All target artifacts for a version must request the same permissions. Signing another target may append an asset to the same release; already published assets cannot be replaced. Use platform-specific targets for native plugins unless all advertised platforms are included in one universal package.
