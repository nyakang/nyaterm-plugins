"""Build the pinned loopback-only VcXsrv runtime with MSVC; never use an installed server."""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
REVISION = "2edd0331433b871c334ff2da6e983f505e2311c4"
REPOSITORY = "https://github.com/nyakang/vcxsrv.git"
TARGET = "x86_64-pc-windows-msvc"
STAGING = ROOT / "target/x11-runtime"
RUNTIME_LAYOUT = 2


def prepare(target: str) -> Path:
    if target != TARGET or sys.platform != "win32":
        raise ValueError("X11 runtime requires a Windows x64 MSVC build host")
    for tool in ("MSBuild.exe", "win_bison.exe", "win_flex.exe", "perl.exe", "nasm.exe", "jom.exe", "gperf.exe", "gzip.exe"):
        if not shutil.which(tool):
            raise RuntimeError("Missing X11 build tool: " + tool)
    source = ROOT / "target/x11-source"
    runtime = STAGING / target
    marker = runtime / "SOURCE.json"
    if marker.is_file():
        identity = json.loads(marker.read_text())
        if identity.get("revision") == REVISION and identity.get("layout") == RUNTIME_LAYOUT:
            return runtime
    source.parent.mkdir(parents=True, exist_ok=True)
    if not (source / ".git").exists():
        subprocess.run(["git", "clone", "--filter=blob:none", "--no-checkout", REPOSITORY, str(source)], check=True)
    subprocess.run(["git", "fetch", "origin", REVISION], cwd=source, check=True)
    subprocess.run(["git", "checkout", "--detach", REVISION], cwd=source, check=True)
    env = os.environ.copy()
    env.update(MHMAKECONF=str(source), PYTHON3=sys.executable, IS64="1", CL=env.get("CL", "") + " /FS")
    jobs = str(min(os.cpu_count() or 2, 8))
    def run(command, directory=source):
        subprocess.run(command, cwd=directory, env=env, check=True)
    # Validate parser generators before spending time on native dependencies.
    run(["MSBuild.exe", "tools/mhmake/mhmakevc10.sln", "-t:Build", "-p:Configuration=Release", "-p:Platform=x64", "-m:" + jobs])
    # Equivalent to upstream buildall.sh, using the caller's MSVC developer
    # environment rather than hard-coded WSL C:\\Python39 and Visual Studio paths.
    run(["MSBuild.exe", "freetype/MSBuild.sln", "-t:Build", "-p:Configuration=Release", "-p:Platform=x64", "-m:" + jobs])
    openssl = source / "openssl/release64"
    openssl.mkdir(exist_ok=True)
    configuration = dict(revision=REVISION, compiler=env.get("VCToolsVersion"), cl=env["CL"])
    configured = openssl / "nyaterm-configuration.json"
    if not configured.is_file() or json.loads(configured.read_text()) != configuration:
        run(["perl.exe", "../Configure", "VC-WIN64A", "--release"], openssl)
        configured.write_text(json.dumps(configuration), encoding="utf-8")
    run(["jom.exe", "/J" + jobs, "build_libs"], openssl)
    run(["nmake.exe", "VC-static"], source / "pthreads")
    run([str(source / "tools/mhmake/Release64/mhmake.exe"), "-P" + jobs, "-C", "xorg-server", "MAKESERVER=1"])
    if runtime.exists():
        shutil.rmtree(runtime)
    runtime.mkdir(parents=True, exist_ok=True)
    # The pinned upstream installer is the authoritative runtime inventory.
    # Exclude tools and user host/config files; retain executable dependencies.
    installer = source / "xorg-server/installer"
    lines = (installer / "vcxsrv-64.nsi").read_text().splitlines()
    for line in lines:
        match = re.fullmatch(r'\s*File "([^"]+)"\s*', line)
        if not match:
            continue
        relative = match[1].replace("\\", "/")
        path = installer / relative
        if path.suffix.lower() not in (".exe", ".dll") or (path.suffix.lower() == ".exe" and path.name not in ("vcxsrv.exe", "xkbcomp.exe")):
            continue
        if not path.is_file() and path.name in ("vcruntime140.dll", "vcruntime140_1.dll", "msvcp140.dll"):
            redist = Path(os.environ["VCToolsRedistDir"])
            found = list(redist.glob("x64/Microsoft.VC*.CRT/" + path.name))
            if len(found) != 1:
                raise RuntimeError("MSVC redistributable is unavailable")
            path = found[0]
        shutil.copy2(path, runtime / path.name)
    for name in ("xkbdata", "locale", "bitmaps", "fonts"):
        path = source / "xorg-server" / name
        if path.is_dir():
            # Upstream makes bitmap fonts optional. Keep misc/scalable fonts,
            # but omit the thousands of legacy 75/100-DPI encoding variants to
            # stay within the existing signed-manifest file-count contract.
            ignore = shutil.ignore_patterns("75dpi", "100dpi") if name == "fonts" else None
            shutil.copytree(path, runtime / name, dirs_exist_ok=True, ignore=ignore)
    for name in ("XKeysymDB", "XtErrorDB", "font-dirs"):
        path = source / "xorg-server" / name
        if path.is_file(): shutil.copy2(path, runtime / name)
    shutil.copy2(source / "libX11/src/XErrorDB", runtime / "XErrorDB")
    shutil.copy2(source / "COPYING", runtime / "COPYING")
    shutil.copy2(source / "NYATERM.md", runtime / "NYATERM.md")
    licenses = runtime / "licenses"
    for path in source.rglob("*"):
        if ".git" not in path.parts and path.is_file() and path.name.upper().startswith(("COPYING", "LICENSE", "NOTICE")):
            destination = licenses / path.relative_to(source)
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, destination)
    (runtime / "NOTICE.md").write_text("VcXsrv and bundled dependencies are independently licensed. See COPYING and licenses/.\nCorresponding source, patches and build instructions are pinned in SOURCE.json and NYATERM.md.\n", encoding="utf-8")
    marker.write_text(json.dumps(dict(repository=REPOSITORY, revision=REVISION, layout=RUNTIME_LAYOUT,
        source=f"https://github.com/nyakang/vcxsrv/archive/{REVISION}.tar.gz",
        build_script="native/scripts/release/prepare_x11.py"), indent=2) + "\n", encoding="utf-8")
    return runtime


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--target", default=TARGET)
    args = parser.parse_args()
    print(prepare(args.target))
