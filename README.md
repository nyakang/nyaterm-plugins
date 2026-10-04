# NyaTerm Plugin Store v1

Official plugin listings, review submissions, publisher records and signed releases for [NyaTerm](https://github.com/nyakang/nyaterm). Plugin source stays in its author's repository. Runtime, SDK, templates and authoring CLI stay in NyaTerm.

## Repository layout

```text
extensions/<plugin-id>/
  listing.json                       # Identity and introduction
  submissions/<version>/<target>.json # Unsigned packages submitted for review
  releases/<version>.json             # Reviewed, signed artifacts for one version
trust/
  publishers/<publisher-id>.json
  signing-keys.json
  revoked.json
tooling/
  commands/                          # Catalog, validation, signing and key setup
  lib/                               # Shared paths, record loading and package checks
  schemas/
  tests/
public/v1/plugins.json                # Generated client catalog; never edit by hand
```

Each extension owns its metadata and release history. `listing.json` contains only ID, name, description, publisher, tags, source, homepage and license. Candidate and release records retain their version-specific permissions; all targets of one version must agree. Directory names and filenames must match record identities.

The catalog derives `verified` from publisher status, `latestVersion` from the highest released SemVer, and top-level permissions from that version. Candidate-only extensions are omitted. Plugin IDs, versions and artifact targets have deterministic ordering, and `generatedAt` comes from release metadata. Empty catalogs retain the initial rollout timestamp.

```sh
npm ci
npm test
npm run catalog
npm run validate
# Download and inspect every submitted and published package:
NYATERM_PLUGIN_CLI=/path/to/nyaterm-plugin npm run validate -- --packages
# Also enable the isolated real pack/sign/validate integration test:
NYATERM_PLUGIN_CLI=/path/to/nyaterm-plugin npm test
```

Commands resolve paths from the repository, independently of the working directory. The public endpoint is:

```text
https://raw.githubusercontent.com/nyakang/nyaterm-plugins/main/public/v1/plugins.json
```

For client interoperability, set `NYATERM_TEST_CATALOG_OUTPUT` to an output file when running Store tests to export the populated catalog fixture. In an updated NyaTerm checkout, set `NYATERM_STORE_CATALOGS` to the public catalog and fixture paths, then run `cargo test --manifest-path src-tauri/crates/nyaterm-plugin-runtime/Cargo.toml --locked --test store_catalog`. Use the OS path-list separator (`:` on Unix, `;` on Windows). This checks both generated catalogs with the client's `Catalog::parse`, without adding a dependency between the repositories.

This reorganization removes the previous catalog endpoint without a compatibility mirror. Older clients cannot load the Store and must upgrade. Publish the Store's new endpoint first, then release the updated NyaTerm client. The catalog remains protocol v1 with repository ID `nyaterm-official`; package formats and installed provenance do not change.

## Review and signing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the listing and submission formats. Source code and unsigned `.nyap` candidates stay in author repositories. Store CI inspects packages with a pinned, reviewed NyaTerm validator; it never builds or executes candidate code.

Signing consumes exactly one reviewed submission, writes or appends its version's release record, and regenerates the catalog. Other submissions, existing artifacts and the listing are preserved. Release tags and asset names are immutable; neither local signed assets nor GitHub Release assets are overwritten. Published package hashes describe the final signed archive, not the unsigned submission.

The client requires HTTPS, bounded downloads, final archive SHA-256 and size, complete checksums, an Ed25519 signature over the exact `checksums.json` bytes, and matching manifest ID/version/publisher/permissions. `signature.json` is excluded from checksums. Publisher verification is separate from cryptographic trust. Unsigned local packages remain supported.

## Initial rollout

1. Land the platform implementation in `nyakang/nyaterm` before enabling Store CI. Both workflows build a fixed reviewed platform commit, never a candidate's source; update that validator ref deliberately.
2. Initialize a key, or pin an existing raw 32-byte Ed25519 public key in base64 in both `trust/signing-keys.json` and the client's `trust.rs`. The initial key list is empty, so Store installs fail closed until configured.
3. Create GitHub Environment `plugin-signing`, require maintainer review, restrict deployment branches to `main`, and put the PKCS#8 PEM private key in its `NYATERM_STORE_SIGNING_KEY` secret. Keep an offline backup. Local initialization restricts the key file to its owner and never uploads secrets.
4. Protect `main` and require Store validation. Enable GitHub Actions to create pull requests or supply a narrowly scoped bot token. Workflow-created PRs using `GITHUB_TOKEN` do not automatically trigger other workflows; validate manually before merge or use an appropriately scoped GitHub App token.
5. Publish the client containing the new catalog address and pinned public key, then submit and sign the first candidate. No sample or unsigned package is advertised as a Store release.

```sh
node tooling/commands/init-signing-key.mjs nyaterm-store-release-2026 /secure/outside-repositories/store.pem ../nyaterm/src-tauri/crates/nyaterm-plugin-runtime/src/trust.rs
```

Signing runs only on `main` in the protected environment, uploads the immutable signed asset, and creates a release/catalog PR for separate review. Client review tokens expire after ten minutes. Installation leaves plugins disabled; runtime permissions require separate authorization.

`trust/revoked.json` records denied artifact hashes and key IDs for publication validation. Remove affected artifacts from release records and regenerate the catalog to stop new installations. Runtime revocation enforcement, third-party repositories and automatic discovery remain future protocol work; this file alone does not disable installed code.
