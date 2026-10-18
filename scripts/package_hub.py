#!/usr/bin/env python3
"""Packages the Oxide Hub download (used by .github/workflows/release.yml).

    scripts/package_hub.py --target aarch64-apple-darwin --version v0.2.0 [--bin-dir DIR] [--out dist]

    macOS    oxide-hub-v0.2.0-<target>.zip     Oxide Hub.app (icon, Info.plist), ad-hoc signed
    Windows  oxide-hub-v0.2.0-<target>.zip     Oxide Hub/oxide-hub.exe (icon embedded at build)
    Linux    oxide-hub-v0.2.0-<target>.tar.gz  oxide-hub/ with the program, icon and a .desktop file

Each gets a .sha256 file next to it. The Hub is small: it downloads editor versions itself.
"""
import argparse, hashlib, os, shutil, subprocess, sys, tarfile, tempfile, zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ASSETS = os.path.join(ROOT, "apps", "oxide_hub", "assets")

INFO_PLIST = """<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Oxide Hub</string>
  <key>CFBundleDisplayName</key><string>Oxide Hub</string>
  <key>CFBundleIdentifier</key><string>dev.oxide.hub</string>
  <key>CFBundleExecutable</key><string>oxide-hub</string>
  <key>CFBundleIconFile</key><string>icon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>{short}</string>
  <key>CFBundleVersion</key><string>{version}</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
</dict>
</plist>
"""

DESKTOP = """[Desktop Entry]
Type=Application
Name=Oxide Hub
Comment=Install Oxide editor versions and open your projects
Exec={exe}
Icon={icon}
Terminal=false
Categories=Development;Game;
"""

LINUX_README = """Oxide Hub

Run ./oxide-hub to start it. To add it to your applications menu:

    ./install.sh

which copies oxide-hub to ~/.local/bin, the icon to ~/.local/share/icons and
oxide-hub.desktop to ~/.local/share/applications.
"""

LINUX_INSTALL = """#!/bin/sh
set -e
here="$(cd "$(dirname "$0")" && pwd)"
mkdir -p "$HOME/.local/bin" "$HOME/.local/share/icons" "$HOME/.local/share/applications"
cp "$here/oxide-hub" "$HOME/.local/bin/oxide-hub"
cp "$here/oxide-hub.png" "$HOME/.local/share/icons/oxide-hub.png"
sed "s|^Exec=.*|Exec=$HOME/.local/bin/oxide-hub|; s|^Icon=.*|Icon=$HOME/.local/share/icons/oxide-hub.png|" \\
    "$here/oxide-hub.desktop" > "$HOME/.local/share/applications/oxide-hub.desktop"
echo "Installed. Find Oxide Hub in your applications menu, or run ~/.local/bin/oxide-hub."
"""


def write(path, text, mode=0o644):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)
    os.chmod(path, mode)


def zip_dir(src_parent, top, archive):
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as z:
        for dirpath, _, files in os.walk(os.path.join(src_parent, top)):
            for name in sorted(files):
                full = os.path.join(dirpath, name)
                arc = os.path.relpath(full, src_parent)
                info = zipfile.ZipInfo.from_file(full, arc)
                info.compress_type = zipfile.ZIP_DEFLATED
                with open(full, "rb") as f:
                    z.writestr(info, f.read())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--target", required=True)
    ap.add_argument("--version", required=True)
    ap.add_argument("--bin-dir", help="folder with the release binary (default target/<target>/release)")
    ap.add_argument("--out", default="dist")
    a = ap.parse_args()

    windows, mac = "windows" in a.target, "apple" in a.target
    exe = "oxide-hub" + (".exe" if windows else "")
    src = os.path.join(a.bin_dir or os.path.join(ROOT, "target", a.target, "release"), exe)
    if not os.path.isfile(src):
        sys.exit(f"missing {src}: build with `cargo build --release -p oxide_hub`")
    version = a.version[1:] if a.version.startswith("v") else a.version
    name = f"oxide-hub-{a.version}-{a.target}"
    os.makedirs(a.out, exist_ok=True)
    stage = tempfile.mkdtemp()

    if mac:
        app = os.path.join(stage, "Oxide Hub.app", "Contents")
        os.makedirs(os.path.join(app, "MacOS"))
        os.makedirs(os.path.join(app, "Resources"))
        shutil.copy2(src, os.path.join(app, "MacOS", "oxide-hub"))
        os.chmod(os.path.join(app, "MacOS", "oxide-hub"), 0o755)
        shutil.copy2(os.path.join(ASSETS, "icon.icns"), os.path.join(app, "Resources", "icon.icns"))
        shutil.copy2(os.path.join(ASSETS, "LICENSE-LUCIDE.txt"), os.path.join(app, "Resources"))
        write(os.path.join(app, "Info.plist"), INFO_PLIST.format(short=version.split("-")[0], version=version))
        if shutil.which("codesign"):
            # Ad-hoc signature over the whole bundle (no Apple developer account needed).
            subprocess.run(["codesign", "--force", "--deep", "-s", "-", os.path.join(stage, "Oxide Hub.app")], check=True)
        archive = os.path.join(a.out, name + ".zip")
        zip_dir(stage, "Oxide Hub.app", archive)
    elif windows:
        top = os.path.join(stage, "Oxide Hub")
        os.makedirs(top)
        shutil.copy2(src, os.path.join(top, exe))
        shutil.copy2(os.path.join(ASSETS, "LICENSE-LUCIDE.txt"), top)
        archive = os.path.join(a.out, name + ".zip")
        zip_dir(stage, "Oxide Hub", archive)
    else:
        top = os.path.join(stage, "oxide-hub")
        os.makedirs(top)
        shutil.copy2(src, os.path.join(top, "oxide-hub"))
        os.chmod(os.path.join(top, "oxide-hub"), 0o755)
        shutil.copy2(os.path.join(ASSETS, "icon.png"), os.path.join(top, "oxide-hub.png"))
        shutil.copy2(os.path.join(ASSETS, "LICENSE-LUCIDE.txt"), top)
        write(os.path.join(top, "oxide-hub.desktop"), DESKTOP.format(exe="oxide-hub", icon="oxide-hub"))
        write(os.path.join(top, "README.txt"), LINUX_README)
        write(os.path.join(top, "install.sh"), LINUX_INSTALL, 0o755)
        archive = os.path.join(a.out, name + ".tar.gz")
        with tarfile.open(archive, "w:gz") as t:
            t.add(top, "oxide-hub")

    digest = hashlib.sha256(open(archive, "rb").read()).hexdigest()
    with open(archive + ".sha256", "w") as f:
        f.write(f"{digest}  {os.path.basename(archive)}\n")
    shutil.rmtree(stage)
    print(f"{archive} ({os.path.getsize(archive) / 1e6:.1f} MB)")


if __name__ == "__main__":
    main()
