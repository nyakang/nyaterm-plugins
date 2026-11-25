import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
import zipfile
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "release"))
from components import inspect, write_package, verify_binaries
from publish import merge
import components
import prepare_android


class ComponentsTests(unittest.TestCase):
    def test_universal_adb_cannot_hide_a_higher_macos_requirement(self):
        import struct
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for version in (12, 13):
                thin = struct.pack("<IiiIIIII", 0xFEEDFACF, 0x1000007, 3, 2, 1, 24, 0, 0)
                thin += struct.pack("<IIIIII", 0x32, 24, 1, version << 16, 0, 0)
                fat = struct.pack(">IIiiIII", 0xCAFEBABE, 1, 0x1000007, 3, 28, len(thin), 0) + thin
                (root / "adb").write_bytes(fat)
                if version == 12:
                    verify_binaries(root, "x86_64-apple-darwin", "macos")
                else:
                    with self.assertRaisesRegex(RuntimeError, "requires macOS 13"):
                        verify_binaries(root, "x86_64-apple-darwin", "macos")

    def test_android_lifecycle_build_does_not_overwrite_production_helper(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            target = "x86_64-pc-windows-msvc"
            production = root / target / "release/nyaterm-android-helper.exe"
            production.parent.mkdir(parents=True)
            production.write_bytes(b"production")
            def cargo(command, **kwargs):
                output = Path(command[command.index("--target-dir") + 1]) if "--target-dir" in command else root
                helper = output / target / "release/nyaterm-android-helper.exe"
                helper.parent.mkdir(parents=True, exist_ok=True)
                helper.write_bytes(b"NYATERM_ANDROID_TEST_ENDPOINT")
            with mock.patch.dict(components.os.environ, {"CARGO_TARGET_DIR": str(root)}), mock.patch.object(sys, "argv", ["components.py", "test", "--component", "android", "--target", target]), mock.patch.object(prepare_android, "prepare", return_value=(root, root)), mock.patch.object(components.subprocess, "run", side_effect=cargo):
                components.main()
            self.assertEqual(production.read_bytes(), b"production")
            with self.assertRaisesRegex(ValueError, "test-support"):
                verify_binaries(root / "android-tests", target, "windows-msvc")

    def test_independent_version_inventory_round_trip_and_tamper_rejection(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            payload = root / "payload"
            payload.mkdir()
            (payload / "nyaterm-vnc-helper.exe").write_bytes(b"controlled helper fixture")
            archive = root / "component.zip"
            write_package(payload, archive, "vnc", "1.2.3", ">=2.0.8, <3", "x86_64-pc-windows-msvc", "windows-msvc", "a" * 40)
            manifest, files = inspect(archive)
            self.assertEqual(manifest["component_version"], "1.2.3")
            self.assertNotIn("app_version", manifest)
            with zipfile.ZipFile(archive, "w") as output:
                for name, data in files.items():
                    output.writestr(name, b"changed" if name.endswith(".exe") else data)
            with self.assertRaises(ValueError):
                inspect(archive)

    def test_architecture_gate_allows_only_bundled_x86_adb_exception(self):
        import struct
        def pe(machine):
            data = bytearray(128)
            data[:2] = b"MZ"
            struct.pack_into("<I", data, 0x3C, 64)
            data[64:68] = b"PE\0\0"
            struct.pack_into("<H", data, 68, machine)
            return data
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            helper = root / "nyaterm-android-helper.exe"
            helper.write_bytes(pe(0x8664))
            (root / "adb").mkdir()
            (root / "adb/adb.exe").write_bytes(pe(0x14C))
            verify_binaries(root, "x86_64-pc-windows-msvc", "windows-msvc")
            helper.write_bytes(pe(0x14C))
            with self.assertRaises(ValueError): verify_binaries(root, "x86_64-pc-windows-msvc", "windows-msvc")

    def test_release_identity_cannot_be_overwritten_and_runtimes_remain_distinct(self):
        first = dict(component="vnc", version="1.0.0", target="x86_64-unknown-linux-gnu", runtime="glibc-2.35", sha256="a" * 64)
        index = merge(dict(schema_version=1, releases=[]), [first])
        self.assertEqual(merge(index, [first]), index)
        with self.assertRaises(ValueError):
            merge(index, [{**first, "sha256": "b" * 64}])
        self.assertEqual(len(merge(index, [{**first, "runtime": "glibc-2.31"}])["releases"]), 2)

    def test_archive_rejects_duplicate_names_and_extra_payload(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            payload = root / "payload"
            payload.mkdir()
            (payload / "nyaterm-vnc-helper.exe").write_bytes(b"fixture")
            archive = root / "component.zip"
            write_package(payload, archive, "vnc", "1.0.0", ">=2.0.8, <3", "x86_64-pc-windows-msvc", "windows-msvc", "a" * 40)
            with zipfile.ZipFile(archive, "a") as output:
                output.writestr("injected.dll", b"unlisted")
            with self.assertRaises(ValueError):
                inspect(archive)


if __name__ == "__main__":
    unittest.main()
