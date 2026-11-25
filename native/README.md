# Official native components

This workspace owns the Android, Builtin RDP, VNC and SPICE helper processes.
Their component versions are independent of NyaTerm. The application consumes
the shared protocol crate at an immutable Git revision; helpers never depend on
the application's workspace or its persisted configuration models.

Native packages retain the official native-component trust boundary, separate
from Wasm extensions. A component is selected by its host version requirement,
IPC version, target and runtime baseline, not by the application's build ID.

The sources in this repository are authoritative. A checkout under another
repository's `temp/` directory is still an independent Git repository, not a
Cargo path dependency of the application.
