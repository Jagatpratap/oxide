#!/usr/bin/env python3
"""Packages an Oxide release download (used by .github/workflows/release.yml).

    scripts/package_release.py --target aarch64-apple-darwin --version v0.2.0 --sdk sdk-out [--bin-dir DIR] [--out dist]

The archive holds the prebuilt editor (from scripts/build_sdk.py), the launcher and the
engine source (player builds compile it):

    oxide-v0.2.0-aarch64-apple-darwin/
        bin/oxide                the project launcher the Hub runs (and the editor sandbox)
        bin/oxide-editor         the project editor: opening a project compiles nothing of the engine
        bin/oxide-mcp-stdio      MCP bridge for stdio-only clients
        lib/                     the engine library the editor runs and Scripts compile against
        sdk/ crates/ apps/ packages/ docs/ Cargo.toml Cargo.lock README.md ...

`bin/oxide` finds the engine source next to itself, so the folder can live anywhere.
Writes a .tar.gz (macOS, Linux) or .zip (Windows) and a .sha256 file next to it.
"""
import argparse, hashlib, os, subprocess, sys, tarfile, zipfile

BINS = ["oxide", "oxide-mcp-stdio"]


def source_files(root):
    """Tracked files at HEAD (what `git archive` would export)."""
    out = subprocess.run(["git", "ls-files", "-z"], cwd=root, check=True, capture_output=True).stdout
    return [p for p in out.decode().split("\0") if p and os.path.isfile(os.path.join(root, p))]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--target", required=True)
    ap.add_argument("--version", required=True)
    ap.add_argument("--bin-dir", help="folder with the release binaries (default target/<target>/release)")
    ap.add_argument("--out", default="dist")
    ap.add_argument("--sdk", required=True, help="output folder of scripts/build_sdk.py")
    a = ap.parse_args()

    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    windows = "windows" in a.target
    exe = ".exe" if windows else ""
    bin_dir = a.bin_dir or os.path.join(root, "target", a.target, "release")
    name = f"oxide-{a.version}-{a.target}"
    os.makedirs(a.out, exist_ok=True)

    entries = []  # (archive path, source path, executable)
    for b in BINS:
        src = os.path.join(bin_dir, b + exe)
        if not os.path.isfile(src):
            sys.exit(f"missing {src}: build with `cargo build --release -p oxide -p oxide_mcp_stdio`")
        entries.append((f"{name}/bin/{b}{exe}", src, True))
    sdk = os.path.abspath(a.sdk)
    if not os.path.isfile(os.path.join(sdk, "lib", "sdk.txt")):
        sys.exit(f"{sdk} is not a build_sdk.py output folder")
    for dirpath, _, files in os.walk(sdk):
        for f in files:
            src = os.path.join(dirpath, f)
            rel = os.path.relpath(src, sdk).replace(os.sep, "/")
            entries.append((f"{name}/{rel}", src, rel.startswith("bin/")))
    for rel in source_files(root):
        entries.append((f"{name}/{rel}", os.path.join(root, rel), os.access(os.path.join(root, rel), os.X_OK)))

    archive = os.path.join(a.out, name + (".zip" if windows else ".tar.gz"))
    if windows:
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as z:
            for arc, src, _ in entries:
                z.write(src, arc)
    else:
        with tarfile.open(archive, "w:gz") as t:
            for arc, src, executable in entries:
                info = t.gettarinfo(src, arc)
                info.mode = 0o755 if executable else 0o644
                info.uid = info.gid = 0
                info.uname = info.gname = ""
                with open(src, "rb") as f:
                    t.addfile(info, f)

    digest = hashlib.sha256(open(archive, "rb").read()).hexdigest()
    with open(archive + ".sha256", "w") as f:
        f.write(f"{digest}  {os.path.basename(archive)}\n")
    size = os.path.getsize(archive) / 1e6
    print(f"{archive} ({size:.1f} MB, {len(entries)} files)")


if __name__ == "__main__":
    main()
