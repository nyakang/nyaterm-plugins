"""Build and validate independently versioned official native component packages."""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import tempfile
try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[2]
IDS = ("android", "rdp", "vnc", "spice", "x11")
PLATFORMS = {
    ("x86_64-pc-windows-msvc", "windows-msvc"),
    ("aarch64-pc-windows-msvc", "windows-msvc"),
    ("x86_64-apple-darwin", "macos"),
    ("aarch64-apple-darwin", "macos"),
    ("x86_64-unknown-linux-gnu", "glibc-2.35"),
    ("aarch64-unknown-linux-gnu", "glibc-2.35"),
    ("x86_64-unknown-linux-gnu", "glibc-2.31"),
}
MAX_ARCHIVE = 256 * 1024 * 1024
MAX_PAYLOAD = 1024 * 1024 * 1024
PUBLIC_KEY = Path(__file__).with_name("updater-public-key.txt")
VERSION = re.compile(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?")


def component_platforms(component):
    return {("x86_64-pc-windows-msvc", "windows-msvc")} if component == "x11" else PLATFORMS


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def ipc_version(component: str) -> int:
    source = ROOT / "crates/nyaterm-native-protocol/src"
    path = source / ({"android": "android/ipc.rs", "spice": "spice/protocol.rs", "x11": "x11.rs"}.get(component, "protocol.rs"))
    name = "X11_PROTOCOL_VERSION" if component == "x11" else "SPICE_PROTOCOL_VERSION" if component == "spice" else "VNC_PROTOCOL_VERSION" if component == "vnc" else "PROTOCOL_VERSION"
    return int(re.search(rf"pub const {name}: u(?:16|32) = ([0-9]+);", path.read_text(encoding="utf-8"))[1])


def validate_manifest(manifest: dict) -> None:
    expected = {"schema_version", "component", "component_version", "host_version", "ipc_version", "runtime", "build_id", "target", "entry", "files"}
    if set(manifest) != expected or manifest["schema_version"] != 2 or manifest["component"] not in IDS:
        raise ValueError("unsupported native manifest")
    if not VERSION.fullmatch(manifest["component_version"]) or not re.fullmatch(r"[0-9a-f]{40}", manifest["build_id"]):
        raise ValueError("invalid component identity")
    if not re.fullmatch(r"[a-zA-Z0-9_.-]+", manifest["runtime"]) or not re.fullmatch(r"[a-zA-Z0-9_-]+", manifest["target"]):
        raise ValueError("invalid component platform")
    if manifest["component"] == "x11" and (manifest["target"], manifest["runtime"]) != ("x86_64-pc-windows-msvc", "windows-msvc"):
        raise ValueError("unsupported X11 platform")
    if (manifest["target"], manifest["runtime"]) not in PLATFORMS:
        raise ValueError("unsupported native runtime")
    entry = "nyaterm-" + manifest["component"] + "-helper" + (".exe" if "windows" in manifest["target"] else "")
    if manifest["entry"] != entry or entry not in manifest["files"] or not 1 <= len(manifest["files"]) <= 4096:
        raise ValueError("invalid entrypoint or inventory")
    seen = set()
    total = 0
    for name, item in manifest["files"].items():
        parts = PurePosixPath(name).parts
        if not parts or name.startswith("/") or "\\" in name or ":" in name or any(p in (".", "..") for p in name.split("/")) or name.lower() in seen or name in ("component.json", "component.minisig"):
            raise ValueError("unsafe native resource path")
        seen.add(name.lower())
        if set(item) != {"size", "sha256", "executable"} or type(item["size"]) is not int or item["size"] < 0 or not re.fullmatch(r"[0-9a-f]{64}", item["sha256"]) or type(item["executable"]) is not bool:
            raise ValueError("invalid resource metadata")
        total += item["size"]
    if total > MAX_PAYLOAD:
        raise ValueError("component payload limit")


def write_package(directory: Path, output: Path, component: str, version: str, host: str, target: str, runtime: str, revision: str) -> dict:
    files = {}
    for path in sorted(directory.rglob("*")):
        if path.is_symlink():
            raise ValueError("symlink in component payload")
        if path.is_file():
            files[path.relative_to(directory).as_posix()] = {"size": path.stat().st_size, "sha256": digest(path.read_bytes()), "executable": bool(path.stat().st_mode & 0o111)}
    manifest = dict(schema_version=2, component=component, component_version=version, host_version=host,
                    ipc_version=ipc_version(component), runtime=runtime, build_id=revision, target=target,
                    entry=f"nyaterm-{component}-helper" + (".exe" if "windows" in target else ""), files=files)
    validate_manifest(manifest)
    output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        archive.writestr("component.json", json.dumps(manifest, sort_keys=True, indent=2).encode() + b"\n")
        for name in files:
            archive.write(directory / name, name)
    if output.stat().st_size > MAX_ARCHIVE:
        raise ValueError("component archive limit")
    return manifest


def inspect(path: Path) -> tuple[dict, dict[str, bytes]]:
    if path.stat().st_size > MAX_ARCHIVE:
        raise ValueError("component archive limit")
    with zipfile.ZipFile(path) as archive:
        entries = archive.infolist()
        if len(entries) > 4098 or sum(e.file_size for e in entries) > MAX_PAYLOAD + 1024 * 1024 + 8192:
            raise ValueError("component expansion limit")
        if len({e.filename.lower() for e in entries}) != len(entries) or any(e.is_dir() or ((e.external_attr >> 16) & 0o170000) not in (0, 0o100000) for e in entries):
            raise ValueError("component duplicate or non-file entry")
        files = {e.filename: archive.read(e) for e in entries}
    if len(files.get("component.json", b"")) > 1024 * 1024:
        raise ValueError("component manifest limit")
    manifest = json.loads(files["component.json"])
    validate_manifest(manifest)
    expected = set(manifest["files"]) | {"component.json"}
    if "component.minisig" in files:
        expected.add("component.minisig")
    if set(files) != expected:
        raise ValueError("unexpected component resources")
    for name, metadata in manifest["files"].items():
        if len(files[name]) != metadata["size"] or digest(files[name]) != metadata["sha256"]:
            raise ValueError("component resource digest mismatch")
    return manifest, files


def sign_bytes(raw: bytes) -> str:
    env = os.environ.copy()
    if encoded := env.pop("TAURI_SIGNING_PRIVATE_KEY_B64", None):
        try:
            env["TAURI_SIGNING_PRIVATE_KEY"] = base64.b64decode(encoded, validate=True).decode()
        except (ValueError, UnicodeError):
            raise ValueError("Invalid signing key encoding") from None
    with tempfile.TemporaryDirectory(prefix="native-sign-") as temporary:
        path = Path(temporary) / "payload"
        path.write_bytes(raw)
        if not env.get("TAURI_SIGNING_PRIVATE_KEY_PASSWORD"):
            raise ValueError("Signing password is required")
        result = subprocess.run([shutil.which("npx") or "npx", "--yes", "@tauri-apps/cli@2.10.0", "signer", "sign",
                                 str(path)], capture_output=True, env=env)
        if result.returncode:
            raise RuntimeError("Native signing failed; signer diagnostics withheld")
        return base64.b64decode(path.with_suffix(".sig").read_bytes().strip(), validate=True).decode()


def verify_signature(raw: bytes, signature: str) -> None:
    with tempfile.TemporaryDirectory(prefix="native-verify-") as temporary:
        root = Path(temporary)
        (root / "payload").write_bytes(raw)
        (root / "signature").write_text(signature, encoding="utf-8")
        subprocess.run(["minisign", "-Vm", str(root / "payload"), "-p", str(PUBLIC_KEY), "-x", str(root / "signature")], check=True, capture_output=True)


def sign_package(path: Path) -> dict:
    manifest, files = inspect(path)
    if "component.minisig" not in files:
        files["component.minisig"] = sign_bytes(files["component.json"]).encode()
        with tempfile.TemporaryDirectory(prefix="native-archive-") as temporary:
            signed = Path(temporary) / "signed.zip"
            with zipfile.ZipFile(signed, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
                for name, data in sorted(files.items()):
                    archive.writestr(name, data)
            shutil.copyfile(signed, path)
    verify_signature(files["component.json"], files["component.minisig"].decode())
    return manifest


def verify_binaries(directory: Path, target: str, runtime: str) -> None:
    from native_headers import pe_machine, elf_machine, macho_cpu_type, verify_macos_minimum_os
    arm = target.startswith("aarch64")
    for path in directory.rglob("*"):
        if not path.is_file(): continue
        data = path.read_bytes()
        if path.name in ("nyaterm-android-helper", "nyaterm-android-helper.exe") and b"NYATERM_ANDROID_TEST_ENDPOINT" in data:
            raise ValueError("test-support Android helper cannot be packaged")
        if data[:2] == b"MZ":
            # Official Windows platform-tools is x86 and runs under Windows emulation.
            expected = 0x14C if path.relative_to(directory).as_posix().startswith("adb/") else (0xAA64 if arm else 0x8664)
            if "windows" not in target or pe_machine(data) != expected:
                raise ValueError("component PE architecture mismatch")
        elif data[:4] == b"\x7fELF":
            if "linux" not in target or elf_machine(data) != (183 if arm else 62):
                raise ValueError("component ELF architecture mismatch")
            output = subprocess.check_output(["readelf", "--version-info", "--wide", str(path)], text=True)
            needs = output.partition("Version needs section")[2]
            if "GLIBC_PRIVATE" in needs:
                raise ValueError("component depends on private glibc ABI")
            floor = tuple(map(int, runtime[len("glibc-"):].split(".")))
            for value in re.findall(r"Name: GLIBC_([0-9.]+)", needs):
                if tuple(map(int, value.split("."))) > floor:
                    raise ValueError("component exceeds its declared glibc baseline")
        elif data[:4] in (b"\xcf\xfa\xed\xfe", b"\xfe\xed\xfa\xcf"):
            if "apple" not in target or macho_cpu_type(data) != (0x100000C if arm else 0x1000007):
                raise ValueError("component Mach-O architecture mismatch")
            verify_macos_minimum_os(data)
        elif data[:4] == b"\xca\xfe\xba\xbe":
            count = int.from_bytes(data[4:8], "big")
            if "apple" not in target or count > 16 or len(data) < 8 + count * 20:
                raise ValueError("invalid universal native binary")
            cpus = {int.from_bytes(data[8+i*20:12+i*20], "big") for i in range(count)}
            if (0x100000C if arm else 0x1000007) not in cpus:
                raise ValueError("universal binary is missing the target architecture")
            for i in range(count):
                offset = int.from_bytes(data[16+i*20:20+i*20], "big")
                size = int.from_bytes(data[20+i*20:24+i*20], "big")
                if offset < 8 + count * 20 or size == 0 or offset + size > len(data):
                    raise ValueError("invalid universal native slice")
                verify_macos_minimum_os(data[offset:offset + size])
        elif path.name.endswith(("-helper", "-helper.exe")):
            raise ValueError("component entrypoint is not a native executable")


def stage(component: str, target: str, destination: Path) -> None:
    suffix = ".exe" if "windows" in target else ""
    build = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")) / target / "release"
    shutil.copy2(build / f"nyaterm-{component}-helper{suffix}", destination)
    shutil.copy2(ROOT / "LICENSE", destination)
    if component == "android":
        import prepare_android
        prepare_android.stage_runtime(prepare_android.STAGING / "adb" / target, prepare_android.STAGING / "ffmpeg" / target, destination)
        prepare_android.write_resource_manifest(destination, target)
        for name in ("NOTICE.md", "LICENSE.scrcpy"):
            shutil.copy2(ROOT / "crates/nyaterm-android-helper/assets" / name, destination)
    elif component == "x11":
        import prepare_x11
        runtime = prepare_x11.prepare(target)
        shutil.copytree(runtime, destination / "vcxsrv")
    elif component == "spice":
        import prepare_spice
        runtime = prepare_spice.prepare(target)
        prepare_spice.verify_runtime(runtime, target)
        shutil.copytree(runtime, destination / "spice")
    if "apple" in target:
        for path in destination.rglob("*"):
            if path.is_file() and (path.name.endswith("-helper") or path.name == "adb" or path.suffix in (".dylib", ".so")):
                subprocess.run(["codesign", "--force", "--sign", "-", "--timestamp=none", str(path)], check=True)
        for relative in ("android-resources.json", "spice/SOURCE.json"):
            path = destination / relative
            if path.exists():
                data = json.loads(path.read_text(encoding="utf-8"))
                resource_root = destination if relative.startswith("android") else destination / "spice"
                data["files"] = {name: digest((resource_root / name).read_bytes()) for name in data["files"]}
                path.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("build", "test", "package", "inspect", "sign"))
    parser.add_argument("--component", choices=IDS)
    parser.add_argument("--target")
    parser.add_argument("--runtime")
    parser.add_argument("--host-version", default=">=2.0.8, <3")
    parser.add_argument("--archive", type=Path)
    args = parser.parse_args()
    if args.operation in ("inspect", "sign"):
        print(json.dumps((sign_package(args.archive) if args.operation == "sign" else inspect(args.archive)[0]), indent=2))
        return
    if not args.component or not args.target:
        parser.error("component and target are required")
    if args.operation == "test":
        environment = os.environ.copy()
        if args.component == "android":
            import prepare_android
            _, ffmpeg = prepare_android.prepare(args.target)
            environment["FFMPEG_DIR"] = str(ffmpeg)
            environment["PATH"] = str(ffmpeg / "bin") + os.pathsep + environment.get("PATH", "")
            environment["LD_LIBRARY_PATH"] = str(ffmpeg / "lib") + os.pathsep + environment.get("LD_LIBRARY_PATH", "")
        command = ["cargo", "test", "--locked", "--release", "--target", args.target, "-p", f"nyaterm-{args.component}-helper"]
        if args.component == "android":
            # Cargo also builds the executable for lifecycle tests. Its optional
            # synthetic endpoint must never overwrite the production package input.
            test_target = Path(environment.get("CARGO_TARGET_DIR", ROOT / "target")) / "android-tests"
            if "apple" in args.target:
                # Cargo rewrites dyld's search environment for test executables.
                # Exercise the helper's packaged @loader_path layout instead.
                prepare_android.stage_runtime(None, ffmpeg, test_target / args.target / "release")
            command += ["--features", "test-support", "--target-dir", str(test_target)]
        subprocess.run(command, cwd=ROOT, env=environment, check=True)
        return
    if args.operation == "build":
        env = os.environ.copy()
        if args.component == "android":
            import prepare_android
            _, ffmpeg = prepare_android.prepare(args.target)
            env["FFMPEG_DIR"] = str(ffmpeg)
        subprocess.run(["cargo", "build", "--locked", "--release", "--target", args.target, "-p", f"nyaterm-{args.component}-helper"], cwd=ROOT, env=env, check=True)
        return
    if not args.runtime or not args.archive:
        parser.error("runtime and archive are required for packaging")
    version = tomllib.loads((ROOT / f"crates/nyaterm-{args.component}-helper/Cargo.toml").read_text())["package"]["version"]
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    with tempfile.TemporaryDirectory(prefix="native-package-") as temporary:
        directory = Path(temporary)
        stage(args.component, args.target, directory)
        verify_binaries(directory, args.target, args.runtime)
        write_package(directory, args.archive, args.component, version, args.host_version, args.target, args.runtime, revision)


if __name__ == "__main__":
    main()
