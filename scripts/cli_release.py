#!/usr/bin/env python3
"""Build and activate the separate Ubuntu CLI installation from tracked inputs."""

import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
from uuid import UUID


ROOT = Path(__file__).resolve().parent.parent
REVISION = re.compile(r"[0-9a-f]{40}(?:[0-9a-f]{24})?")
SHA256 = re.compile(r"[0-9a-f]{64}")
BUILD_INPUTS = (
    "Cargo.toml", "Cargo.lock", "build.rs", ".cargo", "rust-toolchain", "rust-toolchain.toml",
    "src", "crates", "schema", "vendor",
    "docs/document-grouping.md", "docs/personal-memory-grouping.md",
)


def git(*args):
    return subprocess.check_output(["git", "--no-replace-objects", *args], cwd=ROOT)


def binary_sha(binary):
    with binary.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def install_root():
    data_home = Path(os.environ.get("XDG_DATA_HOME", ""))
    if not data_home.is_absolute():
        data_home = Path.home() / ".local/share"
    return data_home / "ontology/cli"


def current_release(root):
    current = root / "current"
    if not current.is_symlink():
        if current.exists():
            raise ValueError("CLI current must be a managed symlink")
        return None
    target = os.readlink(current)
    key = target.removeprefix("releases/")
    if (target != f"releases/{key}" or not SHA256.fullmatch(key)
            or root.is_symlink() or (root / "releases").is_symlink()):
        raise ValueError("CLI current points outside the managed releases")
    return root / target


def inspect_release(release):
    receipt = release / "receipt.json"
    binary = release / "ontology"
    if release.is_symlink() or receipt.is_symlink() or binary.is_symlink():
        raise ValueError("CLI release files must be local regular files")
    if not binary.is_file() or not os.access(binary, os.X_OK) or not receipt.is_file():
        raise ValueError("CLI release is incomplete")
    metadata = json.loads(receipt.read_bytes())
    if (not isinstance(metadata, dict)
            or set(metadata) != {"build_revision", "engine_sha256", "binary_sha256"}
            or not isinstance(metadata["build_revision"], str)
            or not REVISION.fullmatch(metadata["build_revision"])
            or metadata["engine_sha256"] != release.name
            or metadata["binary_sha256"] != binary_sha(binary)):
        raise ValueError("CLI release does not match its receipt")
    return {**metadata, "binary": str(binary)}


def snapshot(inputs, destination):
    # Read the tracked blobs directly. Checkout filters and export attributes
    # cannot make the build consume different bytes from the release key.
    with subprocess.Popen(["git", "--no-replace-objects", "cat-file", "--batch"], cwd=ROOT,
                          stdin=subprocess.PIPE, stdout=subprocess.PIPE) as batch:
        for entry in inputs.rstrip(b"\0").split(b"\0"):
            info, name = entry.split(b"\t", 1)
            mode, kind, oid = info.split()
            if kind != b"blob" or mode not in (b"100644", b"100755"):
                raise ValueError("CLI build inputs must be tracked regular files")
            batch.stdin.write(oid + b"\n")
            batch.stdin.flush()
            header = batch.stdout.readline().split()
            if len(header) != 3 or header[:2] != [oid, b"blob"]:
                raise ValueError("CLI build input blob is unavailable")
            size = int(header[2])
            blob = batch.stdout.read(size)
            if len(blob) != size or batch.stdout.read(1) != b"\n":
                raise ValueError("CLI build input blob is incomplete")
            staged = destination / os.fsdecode(name)
            staged.parent.mkdir(parents=True, exist_ok=True)
            staged.write_bytes(blob)
            staged.chmod(0o700 if mode == b"100755" else 0o600)
        batch.stdin.close()
        if batch.wait() != 0:
            raise ValueError("Git could not materialize the CLI build inputs")


def build_release(stage, inputs, revision, engine_key):
    source = stage / "source"
    snapshot(inputs, source)
    target = stage / "target"
    subprocess.run(["cargo", "build", "--release", "--locked", "--offline", "--bin", "ontology",
                    "--target-dir", str(target)], cwd=source, stdout=sys.stderr, check=True)
    staged = stage / engine_key
    staged.mkdir()
    binary = staged / "ontology"
    shutil.copyfile(target / "release/ontology", binary)
    binary.chmod(0o700)
    receipt = {"build_revision": revision, "engine_sha256": engine_key, "binary_sha256": binary_sha(binary)}
    (staged / "receipt.json").write_text(json.dumps(receipt) + "\n", encoding="utf-8")
    return staged


def verify_store(binary, store_id):
    # Reuse the supported wrapper to resolve the same existing local DB binding.
    env = {**os.environ, "ONTOLOGY_CLI_BINARY": str(binary)}
    result = subprocess.run(["bash", str(ROOT / "scripts/brain.sh"), "context"], cwd=ROOT, env=env,
                            input=b'{"op":"identity"}\n', capture_output=True, timeout=10)
    if result.returncode != 0:
        raise ValueError("Candidate CLI could not verify the configured native store")
    if json.loads(result.stdout) != {"store_id": str(store_id)}:
        raise ValueError("Candidate CLI returned a different native store identity")


def install(root, revision, store_id):
    if not REVISION.fullmatch(revision) or git("rev-parse", "--verify", f"{revision}^{{commit}}").decode().strip() != revision:
        raise ValueError("Select the full ID of a reviewed Git commit")
    inputs = git("ls-tree", "-rz", "--full-tree", revision, "--", *BUILD_INPUTS)
    if not inputs:
        raise ValueError("Selected commit has no CLI build inputs")
    engine_key = hashlib.sha256(inputs).hexdigest()
    root.mkdir(parents=True, exist_ok=True)
    if root.resolve() != root or root.stat().st_uid != os.getuid() or root.stat().st_mode & 0o077:
        raise ValueError("CLI install directory must be a private directory owned by the current user")
    with (root / "install.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        previous = current_release(root)
        releases = root / "releases"
        if releases.is_symlink() or (releases.exists() and not releases.is_dir()):
            raise ValueError("CLI releases must be stored in a local directory")
        release = releases / engine_key
        stage = root / ".staging"
        stage.mkdir()
        try:
            rebuilt = not os.path.lexists(release)
            candidate = build_release(stage, inputs, revision, engine_key) if rebuilt else release
            metadata = inspect_release(candidate)
            verify_store(candidate / "ontology", store_id)
            if rebuilt:
                releases.mkdir(exist_ok=True)
                candidate.rename(release)
            if previous != release:
                switch = stage / "current"
                try:
                    switch.symlink_to(release.relative_to(root))
                    switch.replace(root / "current")
                except OSError:
                    if rebuilt:
                        shutil.rmtree(release)
                    raise
            return {**metadata, "binary": str(release / "ontology"),
                    "requested_revision": revision, "rebuilt": rebuilt}
        finally:
            shutil.rmtree(stage)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("status", help="verify the active executable and print its receipt")
    update = commands.add_parser("install", help="build or reuse a release and verify it before activation")
    update.add_argument("--revision", required=True, help="full ID of the reviewed source commit")
    update.add_argument("--store-id", required=True, type=UUID, help="expected identity of the existing native store")
    args = parser.parse_args()
    if sys.platform != "linux":
        parser.error("the separate CLI installation is supported on Linux")
    os.umask(0o077)
    root = install_root()
    try:
        if args.command == "status":
            release = current_release(root)
            if release is None:
                raise ValueError("No CLI release is active")
            result = inspect_release(release)
        else:
            result = install(root, args.revision, args.store_id)
    except (OSError, ValueError, subprocess.SubprocessError) as exc:
        print(f"Ontology CLI: {exc}", file=sys.stderr)
        return 1
    print(json.dumps(result))
    return 0


if __name__ == "__main__":
    sys.exit(main())
