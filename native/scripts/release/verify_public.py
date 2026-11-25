"""Read-only acceptance of the public signed index and selected release packages."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import tempfile
from urllib.parse import urlparse
from urllib.request import urlopen

from components import IDS, MAX_ARCHIVE, PLATFORMS, digest, inspect, verify_signature

INDEX = "https://downloads.nyaterm.app/native/v1/index.json"


def download(url, destination, limit):
    parsed = urlparse(url)
    if parsed.scheme != "https" or parsed.netloc != "downloads.nyaterm.app" or not parsed.path.startswith("/native/v1/") or parsed.fragment:
        raise ValueError("untrusted native download URL")
    with urlopen(url, timeout=30) as response, destination.open("wb") as output:
        if response.url != url:
            raise ValueError("native downloads must not redirect")
        size = 0
        while chunk := response.read(65536):
            size += len(chunk)
            if size > limit: raise ValueError("native download exceeds limit")
            output.write(chunk)


def verify(components, version):
    with tempfile.TemporaryDirectory(prefix="native-public-") as temporary:
        root = Path(temporary)
        index = root / "index.json"
        download(INDEX, index, 4 * 1024 * 1024)
        envelope = json.loads(index.read_text(encoding="utf-8"))
        verify_signature(envelope["index"].encode(), envelope["signature"])
        catalog = json.loads(envelope["index"])
        if catalog["schema_version"] != 1: raise ValueError("unsupported index")
        releases = [r for r in catalog["releases"] if r["component"] in components and r["version"] == version]
        for component in components:
            selected = [r for r in releases if r["component"] == component]
            if len(selected) != len(PLATFORMS) or {(r["target"], r["runtime"]) for r in selected} != PLATFORMS:
                raise ValueError("incomplete public platform matrix: " + component)
        for release in releases:
            package = root / "component.zip"
            download(release["url"], package, MAX_ARCHIVE)
            if package.stat().st_size != release["size"] or digest(package.read_bytes()) != release["sha256"]:
                raise ValueError("public package digest/size mismatch")
            manifest, files = inspect(package)
            verify_signature(files["component.json"], files["component.minisig"].decode())
            for field, manifest_field in (("component", "component"), ("version", "component_version"),
                                          ("host_version", "host_version"), ("ipc_version", "ipc_version"),
                                          ("target", "target"), ("runtime", "runtime")):
                if release[field] != manifest[manifest_field]:
                    raise ValueError("public index and package disagree: " + field)
        return {"status": "verified", "version": version, "components": components, "packages": len(releases)}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--component", choices=IDS, action="append")
    parser.add_argument("--version", required=True)
    args = parser.parse_args()
    print(json.dumps(verify(args.component or list(IDS), args.version), indent=2))
