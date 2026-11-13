# SPDX-License-Identifier: GPL-3.0-only
r"""Assemble an install set folder for the 0.12 setup wizard (acsetup).

The set format (`actingcommand.install-set.v1`) is the one `crates/acui-wizard/src/set.rs` reads;
the two change together. This script only copies and hashes files and zips the console artifact
folder; it runs nothing it copies. It never deletes or overwrites anything: the output folder
must not exist yet.

Usage:
  python make_install_set.py --out <new folder> --label <text>
      --runtime-dir <folder with the Runtime zip, the Tools zip and SHA256SUMS> --runtime-tag <tag>
      --console-dir <downloaded console artifact folder> --console-tag <tag>
      --bundle <game>=<bundle zip> [--bundle ...]   (a SHA256SUMS beside the zip is checked)
      --vision <path below the vision root>=<file> [--vision ...]
      [--bundle-file <game>=<path below licenses\<game>\>=<file> ...]   (each also listed in the
          bundle's required_files: a set missing one is refused by the wizard)
      [--vision-license <path below licenses\vision\>=<file> ...]   (required when a model is given)
      [--wizard <acsetup.exe>]
"""
import argparse
import hashlib
import json
import os
import re
import shutil
import sys
import zipfile

# The seven pinned vision files of the CPU root (Runtime scripts/windows-tools/
# windows-tool-sources.v1.json; contracts/vision-model-folders.md).
VISION_PINS = {
    "ort/onnxruntime.dll": (17328152, "e7eedec6a6f26dc39dc948276a75ef6d2bee3fff944d874ceed0bbd3b97bff40"),
    "ort/onnxruntime_providers_shared.dll": (22040, "265c8daf29637cb259cac8be9f08f2cd45f3883f0f0e4949cbfddd5b4cbec3b6"),
    "ort/licenses/onnxruntime/LICENSE": (1094, "c250d6278f0b47a6439fb7592b08b58a55eb9f535aa49a1db63211c3f982b674"),
    "ort/licenses/onnxruntime/ThirdPartyNotices.txt": (331175, "fb0af774b4d7cffc5b9d046f2aaeade2f37df2f80abf8033c95dfffcc77a8866"),
    "models/PP-OCRv6_medium/det.onnx": (62032837, "eb13b44b25bb36f89528b68720af8a61d9cf381176107f465db1757b65d086e1"),
    "models/PP-OCRv6_medium/rec.onnx": (76554979, "9c09abf0957f7968c7586464b7397b84ad2387a0497a351af40e9acc71b673ba"),
    "models/PP-OCRv6_medium/keys.txt": (74947, "b5f2bfe2bdd9448429e3e82b51c789775d9b42f2403d082b00662eb77e401c5d"),
}
# DirectML 1.15.4, the file the Runtime pins and the three licence files placed beside it:
# optional as a group, all or none.
DIRECTML_PINS = {
    "ort/DirectML.dll": (18527776, "9c9e6d822561c6c41b90e6994b3e8857cf1d66dbfb1e0c4c799c7c89b4e92da1"),
    "ort/licenses/directml/LICENSE.txt": (10439, "a05138e3a085ff60a44881eedfa58dccb03ecc1d7b1f6ae888418e8c2fec4b8d"),
    "ort/licenses/directml/LICENSE-CODE.txt": (1093, "903df5512f7d02609fed0c780a9b704f5a3eeb6e4d84ebe42a29845c81899a3c"),
    "ort/licenses/directml/ThirdPartyNotices.txt": (4577, "2c95795c13ff48a58b6ed916f37901c23d964b5d9d601af422f17ad2172e7950"),
}
COMMIT = re.compile(r"^[0-9a-f]{40}$")


class Failure(Exception):
    pass


def sha256_of(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def sums_of(folder):
    path = os.path.join(folder, "SHA256SUMS")
    if not os.path.isfile(path):
        return None
    sums = {}
    with open(path, "r", encoding="utf-8") as handle:
        for line in handle:
            line = line.strip()
            if not line:
                continue
            digest, name = line.split(None, 1)
            sums[name.lstrip("*")] = digest.lower()
    return sums


def checked_copy(source, target, expected_sha=None, expected_size=None):
    if os.path.exists(target):
        raise Failure(f"refusing to overwrite {target}")
    os.makedirs(os.path.dirname(target), exist_ok=True)
    shutil.copyfile(source, target)
    size = os.path.getsize(target)
    sha = sha256_of(target)
    if expected_size is not None and size != expected_size:
        raise Failure(f"{source}: {size} bytes, expected {expected_size}")
    if expected_sha is not None and sha != expected_sha:
        raise Failure(f"{source}: sha256 {sha}, expected {expected_sha}")
    return size, sha


def file_ref(out, target, size, sha):
    return {"file": os.path.relpath(target, out).replace(os.sep, "/"), "size": size, "sha256": sha}


def manifest_in_zip(path):
    with zipfile.ZipFile(path) as archive:
        return json.loads(archive.read("BUILD-MANIFEST.json"))


def runtime_entry(out, folder, tag):
    sums = sums_of(folder)
    if sums is None:
        raise Failure(f"{folder}: no SHA256SUMS")
    entry = {"tag": tag}
    commit = None
    for kind in ("runtime", "tools"):
        names = [name for name in sums if name.startswith(f"actingcommand-{kind}-") and name.endswith(".zip")]
        if len(names) != 1:
            raise Failure(f"{folder}: expected one {kind} zip in SHA256SUMS, found {names}")
        name = names[0]
        target = os.path.join(out, "runtime", name)
        size, sha = checked_copy(os.path.join(folder, name), target, expected_sha=sums[name])
        zip_commit = manifest_in_zip(target)["commit_sha"]
        if commit is not None and zip_commit != commit:
            raise Failure(f"{name}: commit {zip_commit} differs from {commit}")
        commit = zip_commit
        entry[f"{kind}_zip"] = file_ref(out, target, size, sha)
    if not COMMIT.match(commit or ""):
        raise Failure(f"runtime commit {commit!r} is not 40 lowercase hex")
    entry["commit"] = commit
    return entry


def console_entry(out, folder, tag):
    manifest_path = os.path.join(folder, "BUILD-MANIFEST.json")
    with open(manifest_path, "r", encoding="utf-8-sig") as handle:
        manifest = json.load(handle)
    commit = manifest["commit_sha"]
    if not COMMIT.match(commit):
        raise Failure(f"console commit {commit!r} is not 40 lowercase hex")
    listed = {item["path"]: item for item in manifest["files"]}
    present = sorted(name for name in os.listdir(folder) if os.path.isfile(os.path.join(folder, name)))
    if sorted(present) != sorted(list(listed) + ["BUILD-MANIFEST.json"]):
        raise Failure(f"{folder}: files {present} differ from the manifest's {sorted(listed)} + BUILD-MANIFEST.json")
    for path, item in listed.items():
        full = os.path.join(folder, path)
        if os.path.getsize(full) != item["size_bytes"] or sha256_of(full) != item["sha256"].lower():
            raise Failure(f"{full} does not match its manifest entry")
    name = f"{manifest.get('source_artifact_name') or 'acui-windows-' + commit}.zip"
    target = os.path.join(out, "console", name)
    if os.path.exists(target):
        raise Failure(f"refusing to overwrite {target}")
    os.makedirs(os.path.dirname(target), exist_ok=True)
    with zipfile.ZipFile(target, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for path in sorted(present):
            archive.write(os.path.join(folder, path), arcname=path)
    return {"tag": tag, "commit": commit, "zip": file_ref(out, target, os.path.getsize(target), sha256_of(target))}


SAFE_PART = re.compile(r"^[A-Za-z0-9._-]+$")


def bundle_files(out, game, pairs):
    """Extra files laid beside a game's bundle (licences, notices): `path` below
    `<root>\\licenses\\<game>\\`, each copied into the set and hashed."""
    files = []
    for path, source in pairs:
        parts = path.split("/")
        if not parts or any(part in ("", ".", "..") or not SAFE_PART.match(part) for part in parts):
            raise Failure(f"{game}: unsafe file path {path!r}")
        target = os.path.join(out, "bundles", game, "files", *parts)
        size, sha = checked_copy(source, target)
        files.append({"path": path, "file": file_ref(out, target, size, sha)["file"], "size": size, "sha256": sha})
    return files


def bundle_entry(out, game, source):
    folder, name = os.path.split(source)
    sums = sums_of(folder)
    expected = sums.get(name) if sums else None
    if sums is not None and expected is None:
        raise Failure(f"{source}: not listed in its SHA256SUMS")
    target = os.path.join(out, "bundles", game, name)
    size, sha = checked_copy(source, target, expected_sha=expected)
    with zipfile.ZipFile(target) as archive:
        bundle = json.loads(archive.read("bundle.json"))
    if bundle.get("game") != game:
        raise Failure(f"{source}: bundle.json names game {bundle.get('game')!r}, not {game!r}")
    return {"game": game, "file": file_ref(out, target, size, sha)["file"], "size": size, "sha256": sha}


def vision_entry(out, pairs, licenses=()):
    files = []
    seen = set()
    pins = {**VISION_PINS, **DIRECTML_PINS}
    for path, source in pairs:
        if path not in pins:
            raise Failure(f"vision path {path} has no pin")
        size, sha = pins[path]
        target = os.path.join(out, "vision", *path.split("/"))
        checked_copy(source, target, expected_sha=sha, expected_size=size)
        ref = file_ref(out, target, size, sha)
        files.append({"path": path, "file": ref["file"], "size": size, "sha256": sha})
        seen.add(path)
    missing = sorted(set(VISION_PINS) - seen)
    if missing:
        raise Failure(f"vision files missing: {missing}")
    directml = set(DIRECTML_PINS) & seen
    if directml and directml != set(DIRECTML_PINS):
        raise Failure(f"DirectML files come all or none; missing: {sorted(set(DIRECTML_PINS) - seen)}")
    entry = {"provider": "cpu", "files": files}
    licence_rows = []
    for path, source in licenses:
        parts = path.split("/")
        if not parts or any(part in ("", ".", "..") or not SAFE_PART.match(part) for part in parts):
            raise Failure(f"vision licence: unsafe path {path!r}")
        target = os.path.join(out, "vision-licenses", *parts)
        size, sha = checked_copy(source, target)
        licence_rows.append({"path": path, "file": file_ref(out, target, size, sha)["file"], "size": size, "sha256": sha})
    if licence_rows:
        entry["licenses"] = licence_rows
    return entry


def pair(text):
    if "=" not in text:
        raise argparse.ArgumentTypeError(f"{text!r} is not <name>=<path>")
    name, path = text.split("=", 1)
    return name, path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", required=True)
    parser.add_argument("--label", required=True)
    parser.add_argument("--runtime-dir", required=True)
    parser.add_argument("--runtime-tag", required=True)
    parser.add_argument("--console-dir", required=True)
    parser.add_argument("--console-tag", required=True)
    parser.add_argument("--bundle", type=pair, action="append", default=[])
    parser.add_argument("--vision", type=pair, action="append", default=[])
    # <game>=<path below licenses\<game>\>=<file>, repeatable.
    parser.add_argument("--bundle-file", type=pair, action="append", default=[])
    # <path below licenses\vision\>=<file>, repeatable (the models' licence texts).
    parser.add_argument("--vision-license", type=pair, action="append", default=[])
    parser.add_argument("--wizard")
    args = parser.parse_args()
    out = os.path.abspath(args.out)
    if any(path.startswith("models/") for path, _ in args.vision) and not args.vision_license:
        raise Failure("a model is given without --vision-license (the wizard refuses a model without its licence)")
    if os.path.exists(out):
        raise Failure(f"{out} exists already; choose a new folder")
    os.makedirs(out)
    document = {
        "schema_version": "actingcommand.install-set.v1",
        "label": args.label,
        "runtime": runtime_entry(out, args.runtime_dir, args.runtime_tag),
        "console": console_entry(out, args.console_dir, args.console_tag),
        "bundles": [bundle_entry(out, game, path) for game, path in args.bundle],
        "vision": vision_entry(out, args.vision, args.vision_license),
    }
    extras = {}
    for game, rest in args.bundle_file:
        if "=" not in rest:
            raise Failure(f"--bundle-file {game}={rest} is not <game>=<path>=<file>")
        path, source = rest.split("=", 1)
        extras.setdefault(game, []).append((path, source))
    for bundle in document["bundles"]:
        pairs = extras.pop(bundle["game"], [])
        if pairs:
            bundle["files"] = bundle_files(out, bundle["game"], pairs)
            # What goes beside a pack here is the licence and attribution of what it carries from
            # elsewhere (an AGPL-3.0 project's, for one): every one of them is one it must not
            # go without, so the wizard refuses a set that lacks one.
            bundle["required_files"] = [path for path, _ in pairs]
    if extras:
        raise Failure(f"--bundle-file names games without a bundle: {sorted(extras)}")
    if args.wizard:
        checked_copy(args.wizard, os.path.join(out, "acsetup.exe"))
    with open(os.path.join(out, "install-set.json"), "x", encoding="utf-8", newline="\n") as handle:
        json.dump(document, handle, ensure_ascii=False, indent=2)
        handle.write("\n")
    print(json.dumps({"out": out, "runtime": document["runtime"]["commit"], "console": document["console"]["commit"],
                      "bundles": [b["game"] for b in document["bundles"]], "vision": len(document["vision"]["files"])},
                     ensure_ascii=False))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Failure as failure:
        print(f"make_set: {failure}", file=sys.stderr)
        sys.exit(1)
