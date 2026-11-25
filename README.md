# NyaTerm Extension Store v2

This repository contains the official v2 extension catalog, reviewed candidates and signing tooling. The Rust SDK, component runtime, native helper registry and package inspector live in NyaTerm. `tooling/v2` and `public/v2/plugins.json` are authoritative; old v1 records and tooling are inactive and are not converted automatically.

## Layout

```text
extensions-v2/<id>/listing.json
extensions-v2/<id>/submissions/<version>.json
trust/publishers/<publisher>.json
trust/revoked.json
public/v2/plugins.json
tooling/v2/
```

A listing contains `id`, `name`, `description`, `publisher`, and `releases`. Releases contain `version`, `host_version`, `api_version`, `capabilities`, and `artifact`. The artifact describes the exact ZIP bytes: HTTPS URL, SHA-256, size and detached Minisign signature. IDs use lowercase letters, numbers, underscores and hyphens; dots are not permitted. `nyaterm-native-` is reserved for application-owned helpers.

```sh
npm ci --ignore-scripts
npm test
npm run catalog
npm run validate
NYATERM_PLUGIN_CLI=/path/to/nyaterm-plugin-tool NYATERM_EXTENSION_PUBLIC_KEY=... npm run validate -- --packages
```

`--packages` checks the generated catalog with the Rust parser, downloads bounded archives, verifies signatures with Minisign, and compares inspected manifests. Inspection compiles and validates the component interface without calling guest exports. Set `NYATERM_TEST_CATALOG_OUTPUT` during tests to export a nonempty interoperability fixture; the same tests invoke the Rust parser when `NYATERM_PLUGIN_CLI` is set.

## Publication

The protected `plugin-signing` environment requires a Minisign private key in `NYATERM_STORE_SIGNING_KEY` and the corresponding raw public key in repository variable `NYATERM_EXTENSION_PUBLIC_KEY`. `NYATERM_VALIDATOR_REF` must be a reviewed 40-character NyaTerm commit. Build clients with the same `NYATERM_EXTENSION_PUBLIC_KEY`. The default client key is empty and store installation fails closed until configured. Native components use a separate application signing key.

The workflow checks out only the reviewed inspector revision, downloads the already-reviewed candidate, signs its exact bytes, uploads immutable assets and opens a catalog PR. It never compiles candidate source or executes guest code. The ZIP digest does not change after signing. No illustrative candidate or unsigned example is published in the catalog.

See CONTRIBUTING.md for records and SECURITY.md for trust and revocation boundaries.
