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
