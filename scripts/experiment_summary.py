#!/usr/bin/env python3
"""Read small decision cards; explicit integrity and retention checks never execute models."""
import argparse
import hashlib
import json
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
            f'실전완료증거={row["real_completion_evidence"]}'))) + " |")
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
               "보관 분류와 용량은 sibling manifest의 retention에 있다. GC는 dry-run만 제공하며 "
               "실제 삭제·이동·압축은 수행하지 않는다.", ""]
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


def retention(root, document, inventory):
    require(root.is_dir(), "Archive unavailable; do not create a fallback")
    policy = inventory["retention"]
    files = index_files(root, inventory)
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


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("render", "integrity", "retention"))
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--manifest", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        require(args.root.is_dir(), "Archive unavailable; no fallback is allowed")
        # Only these two small files are used by the normal path.
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
