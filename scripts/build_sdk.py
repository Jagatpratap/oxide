#!/usr/bin/env python3
"""Builds the prebuilt project editor a release ships (package_release.py --sdk DIR).

    scripts/build_sdk.py --target aarch64-apple-darwin --out sdk-out [--profile release]

Opening a project then compiles nothing of the engine: the editor starts at once and the
project's Scripts compile against the prebuilt engine library (through sdk/oxide_dylib).

    sdk-out/bin/oxide-editor          the project editor (finds its libraries in ../lib)
    sdk-out/lib/liboxide_dylib.dylib  the engine library (.so on Linux; oxide_dylib.dll and its
                                      import library oxide_dylib.dll.lib on Windows)
    sdk-out/lib/libstd-*.dylib        Rust's standard library, which the engine links dynamically
    sdk-out/lib/deps/                 metadata of the crates the engine library is made of, and
                                      its procedural macros: what compiling Scripts against it needs
    sdk-out/lib/sdk.txt               the compiler and target it was built with

On Windows the DLLs are also put next to oxide-editor.exe, where Windows looks for them.
"""
import argparse, glob, json, os, re, shutil, subprocess, sys


def run(cmd, **kw):
    return subprocess.run(cmd, check=True, **kw)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--target", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--profile", default="release")
    a = ap.parse_args()

    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    windows, macos = "windows" in a.target, "apple" in a.target
    out = os.path.abspath(a.out)
    shutil.rmtree(out, ignore_errors=True)
    bin_dir, lib_dir, deps_dir = (os.path.join(out, d) for d in ("bin", "lib", "lib/deps"))
    for d in (bin_dir, deps_dir):
        os.makedirs(d)

    # sdk/oxide_editor is its own workspace (see its Cargo.toml). Starting from the engine's
    # lock file keeps every dependency at the version the engine is tested with.
    editor_crate = os.path.join(root, "sdk", "oxide_editor")
    shutil.copy2(os.path.join(root, "Cargo.lock"), os.path.join(editor_crate, "Cargo.lock"))
    cmd = ["cargo", "build", "--manifest-path", os.path.join(editor_crate, "Cargo.toml"),
           "--profile", a.profile, "--target", a.target, "--message-format=json-render-diagnostics"]
    env = dict(os.environ, CARGO_TARGET_DIR=os.path.join(root, "target"))
    proc = run(cmd, cwd=root, stdout=subprocess.PIPE, env=env)

    editor, engine = None, []
    for line in proc.stdout.decode().splitlines():
        try:
            msg = json.loads(line)
        except ValueError:
            continue
        if msg.get("reason") != "compiler-artifact":
            continue
        kinds, files = msg["target"]["kind"], msg["filenames"]
        if msg.get("executable") and msg["target"]["name"] == "oxide-editor":
            editor = msg["executable"]
        elif "proc-macro" in kinds:
            for f in files:
                shutil.copy2(f, deps_dir)
        elif msg["target"]["name"] == "oxide_dylib":
            engine = [f for f in files if not f.endswith((".rlib", ".rmeta", ".pdb"))]
        elif "custom-build" not in kinds:
            # The compiler needs only the metadata of the crates inside the engine library.
            meta = [f for f in files if f.endswith(".rmeta")] or [f for f in files if f.endswith(".rlib")]
            for f in meta:
                shutil.copy2(f, deps_dir)
    if not editor or not engine:
        sys.exit("cargo did not report oxide-editor and the engine library")

    shutil.copy2(editor, bin_dir)
    for f in engine:
        shutil.copy2(f, lib_dir)

    rustc = os.environ.get("RUSTC", "rustc")
    version = subprocess.check_output([rustc, "--version"], cwd=root).decode().strip()
    sysroot = subprocess.check_output([rustc, "--print", "sysroot"], cwd=root).decode().strip()
    std_pattern = "std-*.dll" if windows else ("libstd-*.dylib" if macos else "libstd-*.so")
    std = glob.glob(os.path.join(sysroot, "lib", "rustlib", a.target, "lib", std_pattern))
    if windows:
        std += glob.glob(os.path.join(sysroot, "bin", std_pattern))
    if not std:
        sys.exit(f"no Rust standard library ({std_pattern}) in {sysroot}")
    for f in std:
        shutil.copy2(f, lib_dir)

    if windows:
        for f in glob.glob(os.path.join(lib_dir, "*.dll")):
            shutil.copy2(f, bin_dir)
    if macos:
        # The engine library is referred to by the path it was built at; make that relative
        # to the editor (its rpath is ../lib) so the install folder can be anywhere.
        lib = os.path.join(lib_dir, "liboxide_dylib.dylib")
        exe = os.path.join(bin_dir, "oxide-editor")
        run(["install_name_tool", "-id", "@rpath/liboxide_dylib.dylib", lib])
        linked = subprocess.check_output(["otool", "-L", exe]).decode()
        old = next(l.split()[0] for l in linked.splitlines()[1:] if "liboxide_dylib" in l)
        run(["install_name_tool", "-change", old, "@rpath/liboxide_dylib.dylib", exe])
        for f in (lib, exe):
            run(["codesign", "--force", "--sign", "-", f], stderr=subprocess.DEVNULL)

    # `rustc 1.99.0 (b940084d7 2026-09-28)` → channel 1.99.0; nightly and beta compilers
    # are named by date instead.
    m = re.match(r"rustc (\S+) \(\S+ (\d{4}-\d\d-\d\d)\)", version)
    if not m:
        sys.exit(f"unrecognized compiler version: {version}")
    ver, date = m.groups()
    channel = f"nightly-{date}" if "nightly" in ver else (f"beta-{date}" if "beta" in ver else ver)
    with open(os.path.join(lib_dir, "sdk.txt"), "w") as f:
        f.write(f"rustc={version}\nchannel={channel}\ntarget={a.target}\n")

    size = sum(os.path.getsize(p) for p in glob.glob(os.path.join(out, "**"), recursive=True) if os.path.isfile(p))
    print(f"{out}: oxide-editor + engine library ({size / 1e6:.0f} MB, {version})")


if __name__ == "__main__":
    main()
