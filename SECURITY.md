# Security policy

Report compromised artifacts or keys through this repository's private security advisory process. Never include private key material or user data in public reports.

V2 uses Minisign signatures over complete ZIP bytes. The client independently checks digest, size, API, ID, publisher, version, host requirement and capabilities. A separate extension-store key cannot authorize official native helper packages. Publisher display metadata alone does not establish trust. Store validation must use a pinned reviewed Rust inspector and must never execute candidate code.

Private signing material belongs in the protected `plugin-signing` environment. Require maintainer approval and restrict signing to main. PR jobs have no signing secrets. Do not run contributed tooling with secrets through `pull_request_target`.

The client persists signed receipts and rechecks their signatures and code identity at activation. Existing resource grants follow the same key/publisher/plugin across authenticated updates. New capabilities and resource scopes require authorization. Local builds use a content identity, so changing code resets grants. Revoking a plugin clears all its previously authorized builds.

`trust/revoked.json` blocks revoked artifact hashes from signing and catalog generation. Remove affected releases, regenerate the catalog and ship an updated client key for a compromised key. This repository does not remotely disable already installed extensions; remediation of installed copies requires a client update or local disable/uninstall. Key rotation currently requires a client release and renewed grants for the new key.

Wasm extensions access resources through declared, scoped host APIs. Authorized local executables run with the user's OS privileges and must be reviewed accordingly. Official protocol helpers remain isolated child processes. Private plugin storage is ordinary local data, not an encrypted credential vault.
