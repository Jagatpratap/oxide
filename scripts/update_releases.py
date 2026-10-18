#!/usr/bin/env python3
"""Adds a published release to releases.json, the list the Oxide Hub reads.

    scripts/update_releases.py --version v0.2.0-beta.1 --repo owner/oxide --dist dist [--file releases.json]

`dist` holds the release archives and their .sha256 files (what scripts/package_release.py
and scripts/package_hub.py write). Editor archives are `oxide-<version>-<target>.<ext>`, Hub
downloads `oxide-hub-<version>-<target>.<ext>`.

The channel comes from the tag: `-alpha...` is alpha, `-beta...` or `-rc...` is beta, anything
else is production. Editing releases.json by hand (to change a channel, or set
"deprecated": true on a broken version) is fine; this script keeps those edits.

releases.json:

    {
      "schema": 1,
      "hub": { "version": "0.2.0", "downloads": { "<target>": { "url", "sha256", "size" } } },
      "releases": [
        { "version": "0.2.0", "channel": "production", "date": "2026-10-18",
          "notes": "https://github.com/owner/oxide/releases/tag/v0.2.0",
          "downloads": { "<target>": { "url", "sha256", "size" } } }
      ]
    }

Releases are kept newest first.
"""
import argparse, datetime, json, os, re, sys

CHANNELS = ("production", "beta", "alpha")


def channel_of(version):
    pre = version.partition("-")[2].lower()
    if not pre:
        return "production"
    if pre.startswith("alpha"):
        return "alpha"
    return "beta"


def version_key(version):
    """Sort key: 0.2.0 > 0.2.0-rc.1 > 0.2.0-beta.2 > 0.2.0-beta.1 > 0.1.9."""
    core, _, pre = version.partition("-")
    nums = tuple(int(n) if n.isdigit() else 0 for n in core.split("."))
    if not pre:
        return (nums, 1, ())
    parts = tuple((0, int(p), "") if p.isdigit() else (1, 0, p) for p in re.split(r"[.\-]", pre))
    return (nums, 0, parts)


def downloads(dist, prefix, tag, repo):
    """{target: {url, sha256, size}} for the archives in `dist` named `<prefix>-<tag>-<target>`."""
    out = {}
    start = f"{prefix}-{tag}-"
    for f in sorted(os.listdir(dist)):
        if not f.startswith(start) or f.endswith(".sha256"):
            continue
        for ext in (".tar.gz", ".zip"):
            if f.endswith(ext):
                target = f[len(start) : -len(ext)]
                break
        else:
            continue
        path = os.path.join(dist, f)
        sha_file = path + ".sha256"
        if not os.path.isfile(sha_file):
            sys.exit(f"missing {sha_file}")
        sha = open(sha_file).read().split()[0]
        out[target] = {
            "url": f"https://github.com/{repo}/releases/download/{tag}/{f}",
            "sha256": sha,
            "size": os.path.getsize(path),
        }
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--version", required=True, help="the tag, e.g. v0.2.0")
    ap.add_argument("--repo", required=True, help="owner/name on GitHub")
    ap.add_argument("--dist", required=True)
    ap.add_argument("--file", default="releases.json")
    ap.add_argument("--date", default=datetime.date.today().isoformat())
    a = ap.parse_args()

    tag = a.version
    version = tag[1:] if tag.startswith("v") else tag
    editor = downloads(a.dist, "oxide", tag, a.repo)
    if not editor:
        sys.exit(f"no editor archives for {tag} in {a.dist}")
    hub = downloads(a.dist, "oxide-hub", tag, a.repo)

    data = {"schema": 1, "releases": []}
    if os.path.isfile(a.file):
        data = json.load(open(a.file))
    old = next((r for r in data["releases"] if r["version"] == version), {})
    entry = dict(old)  # keeps hand edits such as "deprecated"
    entry.update(
        version=version,
        channel=old.get("channel") or channel_of(version),
        date=old.get("date") or a.date,
        notes=f"https://github.com/{a.repo}/releases/tag/{tag}",
        downloads=editor,
    )
    data["releases"] = [r for r in data["releases"] if r["version"] != version] + [entry]
    data["releases"].sort(key=lambda r: version_key(r["version"]), reverse=True)
    # The Hub download offered as an update: the newest production release that has one
    # (or the newest of any channel while there's no production release yet).
    if hub:
        current = data.get("hub", {}).get("version")
        newer = current is None or version_key(version) > version_key(current)
        if newer and (channel_of(version) == "production" or "-" in (current or "-")):
            data["hub"] = {"version": version, "downloads": hub}
    with open(a.file, "w") as f:
        json.dump(data, f, indent=2)
        f.write("\n")
    print(f"{a.file}: {version} ({entry['channel']}), {len(editor)} editor and {len(hub)} Hub downloads")


if __name__ == "__main__":
    main()
