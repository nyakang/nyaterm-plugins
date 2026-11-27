import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "release"))
import publish


class Store:
    def __init__(self, *, old=None, fail_asset=False, conflict=False):
        self.old = old
        self.fail_asset = fail_asset
        self.conflict = conflict
        self.writes = []

    def fetch(self, key, path):
        if self.old is None: return None
        path.write_text(json.dumps(self.old), encoding="utf-8")
        return '"previous-etag"'

    def put(self, key, path, **options):
        self.writes.append((key, options))
        if key != publish.INDEX_KEY and self.fail_asset:
            raise RuntimeError("asset failed")
        if key == publish.INDEX_KEY and self.conflict:
            raise RuntimeError("index conflict")


class PublicationTests(unittest.TestCase):
    def exercise(self, store):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "component.zip").write_bytes(b"fixture")
            record = dict(component="vnc", version="1.0.0", target="x86_64-pc-windows-msvc", runtime="windows-msvc", sha256="a" * 64)
            with mock.patch.object(publish, "release_record", return_value=("native/v1/vnc/component.zip", record)), mock.patch.object(publish, "sign_bytes", return_value="signature"), mock.patch.object(publish, "verify_signature"):
                publish.publish(root, store)

    def test_x11_publication_leaves_legacy_index_untouched(self):
        objects = {}
        class MemoryStore:
            def fetch(self, key, path):
                if key not in objects: return None
                path.write_bytes(objects[key])
                return '"old"'
            def put(self, key, path, **options):
                objects[key] = path.read_bytes()
        old = b'legacy signed envelope'
        objects[publish.INDEX_KEY] = old
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "x11.zip").write_bytes(b"fixture")
            record = dict(component="x11", version="1.0.0", target="x86_64-pc-windows-msvc", runtime="windows-msvc")
            with mock.patch.object(publish, "release_record", return_value=("native/v1/x11/component.zip", record)), mock.patch.object(publish, "sign_bytes", return_value="signature"):
                publish.publish(root, MemoryStore())
        self.assertEqual(objects[publish.INDEX_KEY], old)
        envelope = json.loads(objects["native/v1/x11/index.json"])
        self.assertEqual(json.loads(envelope["index"])["releases"], [record])

    def test_asset_failure_never_replaces_index(self):
        store = Store(fail_asset=True)
        with self.assertRaises(RuntimeError): self.exercise(store)
        self.assertTrue(store.writes)
        self.assertNotIn(publish.INDEX_KEY, [key for key, _ in store.writes])

    def test_index_is_last_and_uses_authenticated_previous_etag(self):
        store = Store(old={"index": json.dumps({"schema_version": 1, "releases": []}), "signature": "old"})
        self.exercise(store)
        self.assertEqual(store.writes[-1], (publish.INDEX_KEY, {"previous": '"previous-etag"'}))
        self.assertEqual(store.writes[0][1], {"immutable": True})

    def test_index_conflict_is_reported_without_retrying_stale_state(self):
        store = Store(conflict=True)
        with self.assertRaises(RuntimeError): self.exercise(store)
        self.assertEqual(sum(key == publish.INDEX_KEY for key, _ in store.writes), 1)

    def test_untrusted_existing_index_cannot_trigger_writes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "component.zip").write_bytes(b"fixture")
            store = Store(old={"index": "{}", "signature": "invalid"})
            with mock.patch.object(publish, "release_record", return_value=("asset", {})), mock.patch.object(publish, "verify_signature", side_effect=ValueError("invalid signature")):
                with self.assertRaises(ValueError): publish.publish(root, store)
            self.assertEqual(store.writes, [])

    def test_r2_index_uses_compare_and_swap_and_reports_conflict(self):
        with mock.patch.dict(publish.os.environ, {"R2_ENDPOINT": "https://abc.r2.cloudflarestorage.com"}):
            store = publish.R2()
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "index.json"
            source.write_bytes(b"{}")
            def fetch(key, destination):
                destination.write_bytes(b"{}")
                return '"etag"'
            with mock.patch.object(store, "call", return_value=mock.Mock(returncode=1)) as call, mock.patch.object(store, "fetch", side_effect=fetch):
                with self.assertRaises(RuntimeError): store.put(publish.INDEX_KEY, source, previous='"old"')
                self.assertIn("--if-match", call.call_args.args)
                self.assertNotIn("--if-none-match", call.call_args.args)

    def test_immutable_retry_accepts_only_identical_signed_bytes(self):
        with mock.patch.dict(publish.os.environ, {"R2_ENDPOINT": "https://abc.r2.cloudflarestorage.com"}):
            store = publish.R2()
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "component.zip"
            source.write_bytes(b"signed original")
            for existing in (b"signed original", b"different signature"):
                def fetch(key, destination):
                    destination.write_bytes(existing)
                    return '"etag"'
                with mock.patch.object(store, "call", return_value=mock.Mock(returncode=1)) as call, mock.patch.object(store, "fetch", side_effect=fetch):
                    if existing == source.read_bytes():
                        store.put("asset", source, immutable=True)
                    else:
                        with self.assertRaises(RuntimeError): store.put("asset", source, immutable=True)
                    self.assertIn("--if-none-match", call.call_args.args)

    def test_retry_after_partial_upload_preserves_previous_component(self):
        objects = {}
        failed = False
        class ResumableStore:
            def fetch(self, key, destination):
                if key not in objects: return None
                destination.write_bytes(objects[key])
                return '"etag"'
            def put(self, key, source, **options):
                nonlocal failed
                if key == "vnc/second" and not failed:
                    failed = True
                    raise RuntimeError("interrupted upload")
                if options.get("immutable") and key in objects:
                    if objects[key] != source.read_bytes(): raise RuntimeError("conflict")
                objects[key] = source.read_bytes()
        previous = dict(component="rdp", version="1.0.0", target="x", runtime="y")
        objects[publish.INDEX_KEY] = json.dumps(dict(index=json.dumps(dict(schema_version=1, releases=[previous])), signature="old")).encode()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for name in ("first", "second"): (root / (name + ".zip")).write_bytes(name.encode())
            def record(path):
                return 'vnc/' + path.stem, dict(component="vnc", version="1.0.0", target=path.stem, runtime="y")
            with mock.patch.object(publish, "release_record", side_effect=record), mock.patch.object(publish, "verify_signature"), mock.patch.object(publish, "sign_bytes", return_value="signed"):
                before = objects[publish.INDEX_KEY]
                with self.assertRaises(RuntimeError): publish.publish(root, ResumableStore())
                self.assertEqual(objects[publish.INDEX_KEY], before)
                self.assertEqual(objects["vnc/first"], b"first")
                publish.publish(root, ResumableStore())
                after = json.loads(json.loads(objects[publish.INDEX_KEY])["index"])
                self.assertEqual(len(after["releases"]), 3)
                self.assertIn(previous, after["releases"])
