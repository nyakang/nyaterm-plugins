# Security policy

Report suspected compromised releases or signing keys privately through GitHub's security advisory reporting for this repository. Never include private signing material in issues, PRs, logs, or attachments.

Repository Ed25519 signatures authenticate reviewed content, not an author's identity or the safety of native code. `verified` publisher metadata is not a cryptographic trust mechanism. Native plugins run with OS user privileges; users must separately review and authorize runtime permissions.

Private keys belong only in the protected `plugin-signing` Environment secret `NYATERM_STORE_SIGNING_KEY`, with maintainer approval and `main` deployment restrictions. PR validation has no signing secrets and never executes plugin source or binaries. Candidate URLs, hashes, manifests and permissions must be approved on `main` before signing. Do not use `pull_request_target` to run submitted Store code.

For rotation, ship the new pinned public key in NyaTerm alongside the old one before switching Store signing. Trusted rotation within the same repository/publisher preserves provenance. Switching repositories or local/Store sources requires uninstalling first. For compromise, revoke the key/hash in `trust/revoked.json`, remove affected artifacts from the extension release records, regenerate the catalog, and release a client that removes compromised pinned keys. Existing installations need incident-specific remediation; v1 does not remotely disable them.
