"""Publish immutable native assets, then atomically replace the signed index."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

from components import digest, inspect, sign_bytes, verify_signature

INDEX_KEY = "native/v1/index.json"


class R2:
    def __init__(self):
        self.bucket = os.environ.get("R2_BUCKET", "nyaterm")
        self.endpoint = os.environ["R2_ENDPOINT"]
        if not re.fullmatch(r"https://[a-zA-Z0-9]+\.r2\.cloudflarestorage\.com", self.endpoint):
            raise ValueError("invalid R2 endpoint")

    def call(self, operation, key, *args):
        return subprocess.run(["aws", "s3api", operation, "--bucket", self.bucket, "--key", key,
                               "--endpoint-url", self.endpoint, *map(str, args)], capture_output=True, text=True, encoding="utf-8")

    def fetch(self, key, destination):
        result = self.call("get-object", key, destination)
        if result.returncode == 0:
            return json.loads(result.stdout)["ETag"]
        if re.search(r"\((?:NoSuchKey|404|NotFound)\)", result.stderr):
            return None
        raise RuntimeError("Native object read failed; absence was not established")

    def put(self, key, source, *, previous=None, immutable=False):
        options = ["--body", source, "--content-type", "application/json" if source.suffix == ".json" else "application/zip",
                   "--cache-control", "public, max-age=31536000, immutable" if immutable else "public, max-age=60, must-revalidate"]
        # R2 evaluates these conditions atomically against committed object state.
        options += ["--if-none-match", "*"] if immutable or previous is None else ["--if-match", previous]
        result = self.call("put-object", key, *options)
        with tempfile.TemporaryDirectory(prefix="native-readback-") as temporary:
            readback = Path(temporary) / "object"
            actual = self.fetch(key, readback)
            if not actual or digest(readback.read_bytes()) != digest(source.read_bytes()):
                raise RuntimeError("Native publish conflicted or readback verification failed")
            if result.returncode and not immutable:
                raise RuntimeError("Native index changed concurrently; retry from a fresh index")


def release_record(path: Path) -> tuple[str, dict]:
    manifest, files = inspect(path)
    verify_signature(files["component.json"], files["component.minisig"].decode())
    key = f"native/v1/{manifest['component']}/{manifest['component_version']}/{manifest['target']}/{manifest['runtime']}/component.zip"
    return key, dict(component=manifest["component"], version=manifest["component_version"], host_version=manifest["host_version"],
                     ipc_version=manifest["ipc_version"], target=manifest["target"], runtime=manifest["runtime"],
                     url="https://downloads.nyaterm.app/" + key, size=path.stat().st_size, sha256=digest(path.read_bytes()))


def identity(release):
    return tuple(release[k] for k in ("component", "version", "target", "runtime"))


def merge(index, releases):
    if set(index) != {"schema_version", "releases"} or index["schema_version"] != 1:
        raise ValueError("unsupported native index")
    entries = {}
    for release in [*index["releases"], *releases]:
        key = identity(release)
        if key in entries and entries[key] != release:
            raise ValueError("published native version cannot be replaced")
        entries[key] = release
    if len(entries) > 10_000:
        raise ValueError("native index capacity exceeded")
    return {"schema_version": 1, "releases": [entries[key] for key in sorted(entries)]}


def publish(directory: Path, store):
    packages = sorted(directory.rglob("*.zip"))
    if not packages:
        raise ValueError("no component packages")
    records = [(path, *release_record(path)) for path in packages]
    with tempfile.TemporaryDirectory(prefix="native-index-") as temporary:
        path = Path(temporary) / "index.json"
        previous = store.fetch(INDEX_KEY, path)
        if previous:
            if path.stat().st_size > 4 * 1024 * 1024:
                raise ValueError("native index size limit")
            envelope = json.loads(path.read_text(encoding="utf-8"))
            verify_signature(envelope["index"].encode(), envelope["signature"])
            index = json.loads(envelope["index"])
        else:
            index = {"schema_version": 1, "releases": []}
        updated = merge(index, [record for _, _, record in records])
        raw = json.dumps(updated, sort_keys=True, separators=(",", ":"))
        envelope = json.dumps({"index": raw, "signature": sign_bytes(raw.encode())}, sort_keys=True) + "\n"
        if len(envelope.encode()) > 4 * 1024 * 1024:
            raise ValueError("native index size limit")
        for package, key, _ in records:
            store.put(key, package, immutable=True)
        path.write_text(envelope, encoding="utf-8")
        store.put(INDEX_KEY, path, previous=previous)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    publish(parser.parse_args().directory, R2())
