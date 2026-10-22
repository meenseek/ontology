#!/usr/bin/env python3
"""Read small decision cards; explicit integrity and retention checks never execute models."""
import argparse
from contextlib import contextmanager
import fcntl
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import sys

from compare_harness import ComparisonError, atomic_write, decode_json, encode, read_bytes, require


def sha(value):
    return hashlib.sha256(encode(value)).hexdigest()


def count(value):
    return type(value) is int and value >= 0


def local_path(root, name):
    require(isinstance(name, str), "Archive path must be a string")
    relative = PurePosixPath(name)
    require(relative.as_posix() == name, "Archive path must use canonical relative spelling")
    require(isinstance(name, str) and name and not relative.is_absolute()
            and ".." not in relative.parts, "Unsafe archive path")
    path = root.joinpath(*relative.parts)
    require(not any(p.is_symlink() for p in [path, *path.parents]
                    if p == root or root in p.parents), "Symlink in archive path")
    return path


def cards(document):
    values = document.get("experiment_decisions")
    require(isinstance(values, list) and values, "No decision cards")
    seen = set()
    for card in values:
        require(isinstance(card, dict), "Invalid card")
        needed = {"id", "question", "execution", "evidence", "improvement", "adoption",
                  "execution_mode", "case_origin", "model_calls", "independent_incidents",
                  "comparable_incidents", "denominator", "result", "limits", "next_action",
                  "bindings", "required_evidence", "capabilities", "measurements"}
        require(needed <= card.keys(), "Incomplete decision card")
        require(isinstance(card["id"], str) and card["id"] not in seen, "Duplicate card")
        seen.add(card["id"])
        require(card["execution"] in {"prepared", "running", "completed", "failed", "interrupted",
                                     "superseded", "invalidated"}, "Unknown execution state")
        require(card["evidence"] in {"sufficient", "limited", "insufficient"}, "Unknown evidence state")
        require(card["improvement"] in {"not_evaluated", "not_established", "improved", "rejected"},
                "Unknown improvement state")
        require(card["adoption"] in {"not_adopted", "adopted", "withdrawn"}, "Unknown adoption state")
        require(card["execution_mode"] in {None, "live", "replay", "mock"}, "Unknown execution mode")
        require(card["case_origin"] in {"real_incident", "synthetic"}, "Unknown case origin")
        for key in ("model_calls", "independent_incidents", "comparable_incidents"):
            require(count(card[key]), "Invalid count")
        require(card["comparable_incidents"] <= card["independent_incidents"], "Invalid comparable count")
        if card["case_origin"] == "synthetic":
            require(card["independent_incidents"] == 0, "Synthetic cases are not real incidents")
        require(isinstance(card["bindings"], dict) and card["bindings"], "Missing version bindings")
        require(isinstance(card["required_evidence"], list), "Missing evidence references")
        require(isinstance(card["capabilities"], dict), "Missing capability declarations")
        denominator = card["denominator"]
        if denominator is not None:
            require(isinstance(denominator, dict), "Invalid denominator")
            required = ("requested", "evaluated", "passed", "failed", "missing", "excluded", "retries")
            require(set(required) == denominator.keys()
                    and all(count(denominator[k]) for k in required), "Invalid denominator")
            require(denominator["requested"] == denominator["evaluated"]
                    + denominator["missing"] + denominator["excluded"]
                    and denominator["evaluated"] == denominator["passed"] + denominator["failed"],
                    "Denominator does not reconcile")
        require(card["evidence"] != "sufficient" or denominator is not None,
                "Exit success alone cannot establish sufficient evidence")
        require(card["execution"] != "prepared" or card["execution_mode"] is None,
                "Preparation is not execution")
    return values


def view(document, state):
    """No evidence paths are opened. State is caller-observed metadata, not a live scan."""
    rows = []
    receipts = document.get("decision_verifications", {})
    for card in cards(document):
        receipt = receipts.get(card["id"], {})
        verified = (receipt.get("status") == "verified_summary"
                    and receipt.get("card_sha256") == sha(card)
                    and bool(receipt.get("verified_at")) and bool(receipt.get("verifier")))
        revoked = card["id"] in state.get("revoked", [])
        bindings = state.get("decision_bindings", {}).get(card["id"])
        applicability = ("revoked" if revoked else "unverified" if not verified else
                         "unknown" if bindings is None else
                         "snapshot_match" if bindings == card["bindings"] else "stale")
        real = (verified and not revoked and applicability == "snapshot_match"
                and card["execution"] == "completed" and card["execution_mode"] == "live"
                and card["case_origin"] == "real_incident" and card["independent_incidents"] > 0
                and card["evidence"] == "sufficient"
                and card["measurements"].get("external_completion_verified") is True)
        rows.append(dict(card, historical_summary_verified=verified,
                         verified_at=receipt.get("verified_at") if verified else None,
                         applicability=applicability, real_completion_evidence=real))
    return rows


def render(document, state):
    rows = view(document, state)
    escape = lambda value: str(value).replace("|", "\\|").replace("\n", " ")
    result = [
        "# 실험 결론 — 판단 카드에서 생성",
        "",
        "정본: completion-outcome.json의 experiment_decisions와 decision_verifications. "
        "이 표는 원장을 읽거나 모델을 실행하지 않고 생성한다.",
        "과거 요약 검증과 현재 적용성은 별개다. snapshot_match는 제공된 메타데이터 시점의 일치이며 "
        "현재 필수 파일 존재나 실전 완료를 다시 검증했다는 뜻이 아니다.",
        "SHA-256은 변경 탐지용이며 진실·서명·위변조 불가능을 보장하지 않는다.",
        "",
        "| 실험 | 목표/비교 | 실행/방식/기원 | 독립/비교 사건·모델 호출 | 결과·판정 | 채택 | 검증·적용성 |",
        "| --- | --- | --- | --- | --- | --- | --- |",
    ]
    for row in rows:
        result.append("| " + " | ".join(map(escape, (
            row["id"], row["question"],
            f'{row["execution"]}/{row["execution_mode"] or "미실행"}/{row["case_origin"]}',
            f'{row["independent_incidents"]}/{row["comparable_incidents"]} · {row["model_calls"]}',
            row["result"] + " / " + row["improvement"], row["adoption"],
            f'요약검증={row["historical_summary_verified"]}; {row["applicability"]}; '
            f'실전완료증거={row["real_completion_evidence"]}'
            + ('; 현재 적용성 재검증 필요' if row['applicability'] == 'stale' else '')))) + " |")
    for row in rows:
        result += ["", f'## {row["id"]}', "",
                   f'증거 충분성: {row["evidence"]}',
                   "분모(실패·누락·제외·재시도 포함): " + escape(json.dumps(row["denominator"], ensure_ascii=False, sort_keys=True)),
                   f'한계: {row["limits"]}', f'다음 조치: {row["next_action"]}',
                   "버전/평가기준: " + escape(json.dumps(row["bindings"], ensure_ascii=False, sort_keys=True)),
                   "최소 재현/재채점 자료: " + ", ".join(row["required_evidence"]),
                   "보유 능력: " + escape(json.dumps(row["capabilities"], ensure_ascii=False, sort_keys=True)),
                   "측정: " + escape(json.dumps(row["measurements"], ensure_ascii=False, sort_keys=True)),
                   f'마지막 요약 검증: {row["verified_at"] or "없음"}']
    policy = document.get("native_policy_status")
    if policy:
        result += ["", "## 기준 반영 상태 (실험 표본 밖)", "",
                   f'Native 적용: {policy["applied"]}; Writer 시도: {policy["attempts"]}; '
                   f'후보: {policy["candidate"]}. {policy["blocker"]}']
    result += ["", "## 조회·보관 경계", "",
               "정상 조회는 이 표 또는 카드·검증 영수증·현재 메타데이터·철회 여부만 읽는다. "
               "새 입력/코드/평가기준, 모순, 재현 요청, 검증 누락이 있을 때만 필요한 증거를 명시적으로 검사한다.",
               "원장 전체 재독은 기본값이 아니다. 재채점·캐시 재생성·과거 실행 재현은 별개 능력이다. "
               "증거가 삭제되면 해당 능력 상실을 먼저 표시하고 과거 검증 기록은 보존한다.",
               "보관 분류와 용량은 sibling manifest의 retention에 있다. 분류는 dry-run이며 "
               "격리·복원은 별도 명시 호출이며 영구 삭제·압축은 제공하지 않는다.", ""]
    return "\n".join(result)


def index_files(root, inventory):
    indexed = {}
    for item in inventory["files"]:
        name = item["path"]
        local_path(root, name)
        require(name not in indexed, "Duplicate inventory path")
        indexed[name] = item
    return indexed


def integrity(root, document, inventory):
    require(root.is_dir(), "Archive unavailable; do not create a fallback")
    indexed = index_files(root, inventory)
    problems = []
    for name in sorted({p for card in cards(document) for p in card["required_evidence"]}):
        path = local_path(root, name)
        expected = indexed.get(name, {}).get("sha256")
        try:
            actual = hashlib.sha256(read_bytes(path)).hexdigest()
        except (OSError, ComparisonError):
            problems.append({"path": name, "status": "missing_or_unreadable"})
            continue
        if not expected or actual != expected:
            problems.append({"path": name, "status": "digest_mismatch"})
    return {"status": "failed" if problems else "verified_required_bytes",
            "problems": problems, "meaning": "Byte integrity only; no truth or model rerun proof"}


def protected_paths(root, document, inventory):
    require(root.is_dir(), "Archive unavailable; do not create a fallback")
    policy = inventory["retention"]
    graph = policy["dependencies"]
    for name, dependencies in graph.items():
        local_path(root, name)
        for dependency in dependencies:
            local_path(root, dependency)
    protected = set()
    pending = [item["path"] for item in policy["roots"]]
    for name in pending:
        local_path(root, name)
    pending += ["completion-outcome.json"]
    pending += [name for card in cards(document) for name in card["required_evidence"]]
    while pending:
        name = pending.pop()
        if name in protected:
            continue
        protected.add(name)
        pending.extend(graph.get(name, []))
    return protected


def retention(root, document, inventory):
    files = index_files(root, inventory)
    protected = protected_paths(root, document, inventory)
    missing = sorted(name for name in protected if not local_path(root, name).is_file())
    groups = {}
    for item in files.values():
        label = item["retention_class"]
        bucket = groups.setdefault(label, {"files": 0, "logical_bytes": 0})
        bucket["files"] += 1
        bucket["logical_bytes"] += item["bytes"]
    candidates = sorted(name for name, item in files.items()
                        if item["retention_class"] == "4" and name not in protected)
    blocked = sorted(name for name, item in files.items()
                     if item["retention_class"] == "4" and name in protected)
    return {"mode": "dry_run", "status": "blocked_missing_roots" if missing else "classified",
            "classes": groups, "protected_files": len(protected), "missing_roots": missing,
            "unreferenced_candidates": candidates, "referenced_candidates": blocked,
            "candidate_logical_bytes": sum(files[n]["bytes"] for n in candidates),
            "immediate_deletion_approved": False, "actual_savings_bytes": 0,
            "physical_recoverable_bytes": None}


@contextmanager
def archive_lock(root, quarantine=None):
    # One cooperating owner; never replace or remove a lock file/inode.
    require(root.is_absolute() and root == root.resolve() and root.is_dir(),
            "Archive must be an existing canonical absolute directory")
    if quarantine is not None:
        require(quarantine.parent == root.parent and quarantine != root
                and quarantine == quarantine.resolve(),
                "Quarantine must be a canonical sibling of the archive")
    descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield
    finally:
        os.close(descriptor)


def sync_directory(path):
    descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def write_receipt(quarantine, receipt):
    atomic_write(quarantine / "receipt.json", encode(receipt))
    sync_directory(quarantine)


def checked_file(path, expected):
    require(path.is_file() and not path.is_symlink(), "Missing or unsafe payload: " + str(path))
    stat = path.stat()
    require(stat.st_size == expected["bytes"]
            and hashlib.sha256(read_bytes(path)).hexdigest() == expected["sha256"],
            "Payload bytes changed: " + str(path))
    return stat


def restore_probe(source, quarantine, expected):
    # A separate inode exercises byte restoration before either source name is removed.
    temporary = local_path(quarantine, ".restore-check")
    if temporary.exists():
        require(temporary.is_file() and temporary.stat().st_nlink == 1,
                "Unsafe abandoned restore probe")
        temporary.unlink()  # reserved regenerable scratch in this owned batch, including partial copies
    try:
        with temporary.open("xb") as target, source.open("rb") as original:
            data = original.read(expected["bytes"] + 1)
            require(len(data) == expected["bytes"], "Payload size changed during restore probe")
            target.write(data)
            target.flush()
            os.fsync(target.fileno())
            os.fsync(original.fileno())
        checked_file(temporary, expected)
    finally:
        temporary.unlink(missing_ok=True)


def read_receipt(root, quarantine):
    receipt = decode_json(read_bytes(local_path(quarantine, "receipt.json")))
    require(receipt["root"] == str(root)
            and (receipt["archive_device"], receipt["archive_inode"])
                == (root.stat().st_dev, root.stat().st_ino)
            and receipt["status"] in
            {"prepared", "quarantined", "restored"}, "Receipt belongs to another archive/state")
    entries = receipt["files"]
    require(isinstance(entries, list) and entries, "Empty receipt")
    names = [entry["path"] for entry in entries]
    require(len(names) == len(set(names)), "Duplicate receipt path")
    for entry in entries:
        local_path(root, entry["path"])
        local_path(quarantine, "files/" + entry["path"])
        require(count(entry["bytes"]) and isinstance(entry["sha256"], str)
                and len(entry["sha256"]) == 64, "Invalid receipt payload")
    return receipt


def payload_pair(root, quarantine, entry):
    source = local_path(root, entry["path"])
    stored = local_path(quarantine, "files/" + entry["path"])
    require(source.exists() or stored.exists(), "Both payload locations are missing")
    for path in (source, stored):
        if path.exists():
            stat = checked_file(path, entry)
            require(stat.st_dev == entry["device"] and stat.st_ino == entry["inode"],
                    "Payload identity changed: " + str(path))
    if source.exists() and stored.exists():
        require(source.samefile(stored), "Destination collision; never overwrite")
    return source, stored


def current_inputs(root, manifest):
    document_bytes = read_bytes(local_path(root, "completion-outcome.json"))
    manifest_bytes = read_bytes(manifest)
    return (decode_json(document_bytes), decode_json(manifest_bytes),
            hashlib.sha256(document_bytes).hexdigest(), hashlib.sha256(manifest_bytes).hexdigest())


def quarantine_files(root, manifest, quarantine, selected, expected_document, expected_manifest):
    """Explicit duplicates only. No recipe execution, permanent deletion, or index rewriting."""
    with archive_lock(root, quarantine):
        document, inventory, document_digest, manifest_digest = current_inputs(root, manifest)
        require((document_digest, manifest_digest) == (expected_document, expected_manifest),
                "Archive metadata changed; review the new selection")
        report = retention(root, document, inventory)
        require(report["status"] == "classified" and integrity(root, document, inventory)["status"]
                == "verified_required_bytes", "Protected evidence is unavailable or changed")
        require(selected and len(selected) == len(set(selected)), "Select unique candidate paths explicitly")
        require(set(selected) <= set(report["unreferenced_candidates"]), "Selected path is protected or not a candidate")
        protected = protected_paths(root, document, inventory)
        files = index_files(root, inventory)
        entries = []
        for name in sorted(selected):
            item = files[name]
            canonical = item.get("canonical_duplicate_of")
            require(canonical in protected and canonical not in selected,
                    "Candidate needs an independently retained canonical duplicate")
            require(canonical in files and files[canonical]["sha256"] == item["sha256"],
                    "Canonical duplicate binding is missing or different")
            checked_file(local_path(root, canonical), item)
            entries.append(dict(path=name, bytes=item["bytes"], sha256=item["sha256"]))
        new_batch = not quarantine.exists()
        if not new_batch:
            receipt = read_receipt(root, quarantine)
            require(receipt["status"] != "restored", "Restored batch cannot be quarantined again")
            require(receipt["document_sha256"] == document_digest
                    and receipt["manifest_sha256"] == manifest_digest
                    and [{k:entry[k] for k in ("path", "bytes", "sha256")}
                         for entry in receipt["files"]] == entries, "Receipt selection changed")
        else:
            for entry in entries:
                stat = checked_file(local_path(root, entry["path"]), entry)
                require(stat.st_nlink == 1 and stat.st_dev == root.stat().st_dev,
                        "Source must be an unaliased file on the archive device")
                entry.update(device=stat.st_dev, inode=stat.st_ino)
            receipt = dict(root=str(root), archive_device=root.stat().st_dev,
                           archive_inode=root.stat().st_ino, document_sha256=document_digest,
                           manifest_sha256=manifest_digest, status="prepared", files=entries)
        protected_identities = set()
        for name in protected:
            stat = local_path(root, name).stat()
            protected_identities.add((stat.st_dev, stat.st_ino))
        selected_identities = [(entry["device"], entry["inode"]) for entry in receipt["files"]]
        require(not protected_identities.intersection(selected_identities),
                "Selected filesystem entry is protected through another path")
        require(len(selected_identities) == len(set(selected_identities)),
                "Selected paths alias the same filesystem entry")
        if new_batch:
            quarantine.mkdir(mode=0o700)
            write_receipt(quarantine, receipt)
            sync_directory(quarantine.parent)
        require(quarantine.stat().st_dev == root.stat().st_dev, "Cross-device quarantine is unsupported")
        # Preflight the whole batch before modifying any source name, including on resume.
        for entry in receipt["files"]:
            payload_pair(root, quarantine, entry)
        for entry in receipt["files"]:
            source, stored = payload_pair(root, quarantine, entry)
            if not stored.exists():
                stored.parent.mkdir(parents=True, exist_ok=True)
                os.link(source, stored, follow_symlinks=False)  # fails rather than overwriting
                sync_directory(stored.parent)
            checked_file(stored, entry)
            restore_probe(stored, quarantine, entry)
        require(integrity(root, document, inventory)["status"] == "verified_required_bytes",
                "Protected evidence changed during restore probes")
        for entry in receipt["files"]:
            require(current_inputs(root, manifest)[2:] == (document_digest, manifest_digest),
                    "Archive metadata changed during quarantine; restore before reselection")
            checked_file(local_path(root, files[entry["path"]]["canonical_duplicate_of"]), entry)
            source, stored = payload_pair(root, quarantine, entry)
            if source.exists():
                require(source.stat().st_nlink == 2, "Unexpected additional payload alias")
                source.unlink()
                sync_directory(source.parent)
        receipt["status"] = "quarantined"
        write_receipt(quarantine, receipt)
        return dict(status="quarantined", files=len(receipt["files"]),
                    restore_verified=True, permanent_deleted_files=0, actual_savings_bytes=0)


def restore_files(root, quarantine):
    """Restore without overwriting; also recovers any prepared/partially moved batch."""
    with archive_lock(root, quarantine):
        receipt = read_receipt(root, quarantine)
        for entry in receipt["files"]:
            source, stored = payload_pair(root, quarantine, entry)
            require(source.parent.is_dir(), "Original parent is missing; resolve before restoring")
            if stored.exists():
                restore_probe(stored, quarantine, entry)
        for entry in receipt["files"]:
            source, stored = payload_pair(root, quarantine, entry)
            if not source.exists():
                os.link(stored, source, follow_symlinks=False)
                sync_directory(source.parent)
            checked_file(source, entry)
            if stored.exists():
                require(source.samefile(stored), "Restore collision")
                stored.unlink()
                sync_directory(stored.parent)
        receipt["status"] = "restored"
        write_receipt(quarantine, receipt)
        return dict(status="restored", files=len(receipt["files"]), permanent_deleted_files=0)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("render", "integrity", "retention", "quarantine", "restore"))
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--manifest", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--quarantine", type=Path, help="Explicit batch directory beside the archive")
    parser.add_argument("--select", action="append", help="Exact candidate path; repeat for each selected file")
    parser.add_argument("--expected-document-sha256")
    parser.add_argument("--expected-manifest-sha256")
    args = parser.parse_args(argv)
    try:
        require(args.root.is_dir(), "Archive unavailable; no fallback is allowed")
        if args.action in {"quarantine", "restore"}:
            require(args.quarantine is not None and args.output is None, "Explicit quarantine directory required; no output file allowed")
            if args.action == "quarantine":
                require(args.manifest is not None, "Explicit manifest is required")
                result = quarantine_files(args.root, args.manifest, args.quarantine, args.select,
                                          args.expected_document_sha256, args.expected_manifest_sha256)
            else:
                require(args.select is None and args.manifest is None
                        and args.expected_document_sha256 is None and args.expected_manifest_sha256 is None,
                        "Restore uses its receipt; selection and manifest options are not accepted")
                result = restore_files(args.root, args.quarantine)
            sys.stdout.buffer.write(encode(result))
            return 0
        require(args.quarantine is None and args.select is None
                and args.expected_document_sha256 is None and args.expected_manifest_sha256 is None,
                "Mutation options require quarantine or restore")
        # Only this small file is used by the normal path.
        document = decode_json(read_bytes(local_path(args.root, "completion-outcome.json")))
        if args.action != "render":
            require(args.manifest is not None, "Explicit manifest is required for evidence checks")
            manifest = decode_json(read_bytes(args.manifest))
        if args.action == "render":
            result = render(document, document.get("current_state", {})).encode()
        elif args.action == "integrity":
            result = encode(integrity(args.root, document, manifest))
        else:
            result = encode(retention(args.root, document, manifest))
        if args.output:
            require(args.action == "render" and args.output.name == "README.md",
                    "Only the generated README.md may be written")
            protected = {local_path(args.root, "completion-outcome.json").resolve()}
            protected.update(local_path(args.root, name).resolve()
                             for card in cards(document) for name in card["required_evidence"])
            if args.manifest is not None:
                protected.add(args.manifest.resolve())
            require(args.output.resolve() not in protected, "Output collides with retained input")
            require(args.output.parent.resolve() == args.root.resolve(),
                    "Output must stay in the existing archive root")
            with archive_lock(args.root.resolve()):
                atomic_write(args.output, result)
        else:
            sys.stdout.buffer.write(result)
        return int(args.action != "render" and
                   decode_json(result).get("status") in {"failed", "blocked_missing_roots"})
    except (ComparisonError, KeyError, TypeError, OSError) as error:
        print(json.dumps({"status": "unavailable" if not args.root.is_dir() else "invalid",
                          "error": str(error)}, ensure_ascii=False), file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
