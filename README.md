# NyaTerm Plugin Store v1

Official plugin metadata, publisher records, review candidates, repository keys and signed release artifacts for [NyaTerm](https://github.com/nyakang/nyaterm). Third-party source stays in its author's repository. Runtime, SDK, templates, and authoring CLI stay in NyaTerm.

`plugins/*.json` is the source of truth. `catalog/index.json` is generated deterministically; do not edit it by hand. Candidate-only records are omitted from the public catalog. Permissions are recorded **per version** as well as on the latest plugin record, so permission changes and older versions can coexist.

```sh
npm ci
npm test
npm run catalog
npm run validate
# Also download and inspect every candidate and published package:
NYATERM_PLUGIN_CLI=/path/to/nyaterm-plugin npm run validate -- --packages
```

The client requires HTTPS, a bounded download, the final archive SHA-256, complete checksums, an Ed25519 signature over the **exact `checksums.json` bytes**, and matching manifest ID/version/publisher/permissions. Publisher verification is a separate identity label. `signature.json` is excluded from checksums to avoid circular signing. Unsigned local packages remain supported.

## Initial rollout

1. Land the platform implementation in `nyakang/nyaterm` before enabling Store CI. CI builds the pinned, reviewed platform commit in both workflows, never a candidate's source. Advance these checkout refs deliberately when updating the platform validator.
2. Initialize a key using the script below, or pin an existing **raw 32-byte Ed25519 public key in base64** in both `signing-keys.json` and the client's `trust.rs`. The initial trust store is empty and Store installs fail closed until configured.
3. Create GitHub Environment `plugin-signing`, require maintainer review, limit deployment branches to `main`, and put the PKCS#8 PEM private key in its `NYATERM_STORE_SIGNING_KEY` secret. Keep an offline backup. The initializer restricts the local key file to its owner (Windows ACL or POSIX mode 0600) and never uploads secrets.
4. Protect `main` and require Store validation. Enable GitHub Actions to create pull requests (or supply a narrowly scoped bot token); review the resulting catalog PR and merge it. Workflow-created PRs using `GITHUB_TOKEN` do not automatically trigger other workflows, so run validation manually before merge or use an appropriately scoped GitHub App token.
5. Publish the client containing that public key, then submit and sign the first candidate. No sample or unsigned package is advertised as a Store release.

```sh
node scripts/init-signing-key.mjs nyaterm-store-release-2026 /secure/outside-repositories/store.pem ../nyaterm/src-tauri/crates/nyaterm-plugin-runtime/src/trust.rs
```

Signing runs only on `main` in the protected environment. It verifies the reviewed candidate SHA and manifest, inserts `signature.json`, verifies the result, computes the final archive digest, uploads an immutable GitHub Release asset, and creates a metadata/catalog PR. Assets are never overwritten. Review tokens in the client expire after ten minutes and install disabled; runtime permissions need separate authorization.

`revoked.json` records denied artifact hashes and key IDs for publication validation. Remove affected artifacts from metadata and regenerate the catalog to stop new installations. Client-side runtime revocation enforcement, third-party repositories and automatic release discovery are intentionally future protocol work; do not treat this file alone as disabling already installed code.
