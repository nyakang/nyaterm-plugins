# Submitting a v2 extension

Build against `nyaterm-plugin-api` 2.0.0 in the NyaTerm repository. Use the examples under `examples/plugins`, then run `python scripts/plugins/build.py`. Package a directory with `nyaterm-plugin-tool pack <directory> <output.zip>`; `inspect <output.zip>` returns protocol version 2 and the validated manifest. Inspection does not initialize the guest.

Add a publisher record under `trust/publishers/` and a listing under `extensions-v2/example-tools/listing.json`:

```json
{
  "id": "example-tools",
  "name": "Example tools",
  "description": "Tools for SSH sessions",
  "publisher": "example",
  "releases": []
}
```

Submit `extensions-v2/example-tools/submissions/1.0.0.json`:

```json
{
  "url": "https://example.com/releases/example-tools-1.0.0.zip",
  "sha256": "replace-with-the-exact-64-character-lowercase-archive-digest",
  "size": 123456,
  "host_version": ">=2.0.8",
  "api_version": "2.0.0",
  "capabilities": ["ui", "remote_files_read"]
}
```

Use the actual HTTPS asset URL, digest and size. The directory, version filename, manifest ID/version/publisher, API, host requirement and capabilities must match. Host requirements use Rust semver syntax; separate multiple comparators with commas. Keep release versions immutable. Native protocol decoder helpers cannot be submitted as third-party Wasm extensions.

Run `npm run catalog`, `npm run validate`, and `npm test`. Reviewers check the source, dependency provenance, license, requested scopes, command execution, remote scripts and release archive. After review and merge, a maintainer dispatches the signing workflow on protected main. The signed catalog change receives a separate review PR. A signature establishes repository approval; users still authorize external resources in the client.
