#!/usr/bin/env python3
"""Run scaled D2B1-R loopback admission evidence without external network access."""

from __future__ import annotations

import argparse
import ctypes
import json
import os
import platform
import re
import statistics
import subprocess
import sys
import sysconfig
import tempfile
import time
from collections import Counter, defaultdict
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

SCHEMA_VERSION = "d2b-admission-scaled-v1"
BASE_SEED = 20_260_723
SEED_MIX_MULTIPLIER = 0x9E3779B97F4A7C15
U64_MASK = (1 << 64) - 1
CANDIDATES = ["compatibility", "g2-h2", "g4-h1", "g4-h2", "g4-h4", "g8-h2", "g8-h4"]
RANKED_SCENARIOS = [
    "single_origin_sustained",
    "single_origin_pressure",
    "multi_origin_sustained",
    "multi_origin_mixed_rate",
    "same_origin_mixed_size",
]
SAFETY_SCENARIOS = [
    "queue_soak",
    "resolver_refresh",
    "redirect_same_origin",
    "redirect_other_origin",
    "backend_abnormal",
]
PRESSURE_SCENARIO = "single_origin_pressure"
NON_PRESSURE_SCENARIOS = [scenario for scenario in RANKED_SCENARIOS if scenario != PRESSURE_SCENARIO]
MIN_SCENARIO_DURATION_NS = 750_000_000
EXPECTED_DEFAULT_PROFILE = {
    "max_attempts": 3,
    "backoff_ms": 2_000,
    "timeout_ms": 60_000,
    "chunk_size": 1024 * 1024,
}
FORBIDDEN_EVIDENCE_PATTERNS = (
    r"(?i)https?://",
    r"(?i)127\.0\.0\.1",
    r"(?i)localhost",
    r"(?i)authorization\s*[:=]",
    r"(?i)cookie\s*[:=]",
    r"(?i)bearer\s+",
    r"(?i)\btoken\s*[:=]",
    r"(?i)\b[A-Z]:[\\/]",
)
SCENARIO_DEFINITIONS = {
    "single_origin_sustained": {
        "origins": 1,
        "files": 4,
        "total_bytes": 24 * 1024 * 1024,
        "server_chunk_bytes": 64 * 1024,
        "delay_ms": [10],
    },
    "single_origin_pressure": {
        "origins": 1,
        "files": 8,
        "total_bytes": 24 * 1024 * 1024,
        "server_chunk_bytes": 64 * 1024,
        "delay_ms": [16],
        "pressure_threshold": 2,
        "failure_plan": "first request above threshold returns one alternating 429 or 500",
    },
    "multi_origin_sustained": {
        "origins": 4,
        "files": 8,
        "total_bytes": 48 * 1024 * 1024,
        "server_chunk_bytes": 64 * 1024,
        "delay_ms": [10],
    },
    "multi_origin_mixed_rate": {
        "origins": 4,
        "files": 8,
        "total_bytes": 40 * 1024 * 1024,
        "server_chunk_bytes": 64 * 1024,
        "delay_ms": [12, 14, 16, 20],
    },
    "same_origin_mixed_size": {
        "origins": 1,
        "files": 3,
        "total_bytes": 20 * 1024 * 1024,
        "server_chunk_bytes": 64 * 1024,
        "delay_ms": [10],
    },
}


class BenchmarkFailure(RuntimeError):
    """A bounded benchmark protocol failure safe to show to the caller."""


@dataclass(frozen=True)
class BatchInvocation:
    round_number: int
    seed: int
    warmups: int
    measured_repetitions: int


def effective_seed(invocation: BatchInvocation) -> int:
    return invocation.seed ^ (
        (invocation.round_number * SEED_MIX_MULTIPLIER) & U64_MASK
    )


class ProcessMemoryCounters(ctypes.Structure):
    _fields_ = [
        ("cb", ctypes.c_ulong),
        ("PageFaultCount", ctypes.c_ulong),
        ("PeakWorkingSetSize", ctypes.c_size_t),
        ("WorkingSetSize", ctypes.c_size_t),
        ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
        ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
        ("PagefileUsage", ctypes.c_size_t),
        ("PeakPagefileUsage", ctypes.c_size_t),
    ]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--expected-branch", required=True)
    parser.add_argument("--expected-commit", required=True)
    parser.add_argument("--output-json", required=True, type=Path)
    parser.add_argument("--output-markdown", required=True, type=Path)
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--rounds", type=int, default=2)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--repetitions", type=int, default=4)
    parser.add_argument("--seed", type=int, default=BASE_SEED)
    parser.add_argument("--replace", action="store_true")
    args = parser.parse_args()
    if args.rounds != 2:
        parser.error("--rounds is frozen at 2")
    if args.warmups != 1:
        parser.error("--warmups is frozen at 1")
    if args.repetitions != 4:
        parser.error("--repetitions is frozen at 4")
    if args.seed <= 0:
        parser.error("--seed must be positive")
    return args


def run_text(command: list[str], cwd: Path) -> str:
    completed = subprocess.run(
        command,
        cwd=cwd,
        check=False,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    if completed.returncode != 0:
        raise BenchmarkFailure("required local command failed")
    return completed.stdout.strip()


def verify_context(root: Path, expected_branch: str, expected_commit: str) -> None:
    branch = run_text(["git", "branch", "--show-current"], root)
    commit = run_text(["git", "rev-parse", "HEAD"], root)
    if branch != expected_branch or commit != expected_commit:
        raise BenchmarkFailure("branch or commit does not match the supplied benchmark context")


def build_example(root: Path) -> Path:
    run_text(
        ["cargo", "build", "--offline", "--release", "-p", "insar-download", "--example", "admission_benchmark"],
        root,
    )
    target_root = Path(os.environ.get("CARGO_TARGET_DIR", root / "target"))
    binary = target_root / "release" / "examples" / "admission_benchmark"
    if os.name == "nt":
        binary = binary.with_suffix(".exe")
    if not binary.is_file():
        raise BenchmarkFailure("release benchmark binary was not produced")
    return binary


def windows_process_metrics(pid: int) -> tuple[int | str, int | str]:
    if os.name != "nt":
        return "unavailable", "unavailable"
    process_query_information = 0x0400
    process_vm_read = 0x0010
    handle = ctypes.windll.kernel32.OpenProcess(process_query_information | process_vm_read, False, pid)
    if not handle:
        return "unavailable", "unavailable"
    try:
        counters = ProcessMemoryCounters()
        counters.cb = ctypes.sizeof(counters)
        if not ctypes.windll.psapi.GetProcessMemoryInfo(
            handle, ctypes.byref(counters), ctypes.sizeof(counters)
        ):
            peak_memory: int | str = "unavailable"
        else:
            peak_memory = int(counters.PeakWorkingSetSize)
        created = ctypes.c_ulonglong()
        exited = ctypes.c_ulonglong()
        kernel = ctypes.c_ulonglong()
        user = ctypes.c_ulonglong()
        if not ctypes.windll.kernel32.GetProcessTimes(
            handle,
            ctypes.byref(created),
            ctypes.byref(exited),
            ctypes.byref(kernel),
            ctypes.byref(user),
        ):
            cpu_time: int | str = "unavailable"
        else:
            cpu_time = int((kernel.value + user.value) // 10_000)
        return cpu_time, peak_memory
    finally:
        ctypes.windll.kernel32.CloseHandle(handle)


def sanitize_stderr(value: str) -> str:
    return "" if not value.strip() else "[diagnostic redacted]"


def ensure_safe_text(value: str) -> None:
    if any(re.search(pattern, value) for pattern in FORBIDDEN_EVIDENCE_PATTERNS):
        raise BenchmarkFailure("benchmark evidence contains a forbidden sensitive marker")


def numeric(value: Any) -> int | float | None:
    return value if isinstance(value, (int, float)) and not isinstance(value, bool) else None


def robust_summary(values: list[int | float]) -> dict[str, int | float]:
    if not values:
        raise BenchmarkFailure("benchmark aggregation received no values")
    ordered = sorted(values)
    median = statistics.median(ordered)
    deviations = [abs(value - median) for value in ordered]
    return {
        "count": len(ordered),
        "min": ordered[0],
        "max": ordered[-1],
        "median": median,
        "median_absolute_deviation": statistics.median(deviations),
    }


def profile_is_production_default(profile: Any) -> bool:
    if not isinstance(profile, dict):
        return False
    options = profile.get("transfer_options")
    return isinstance(options, dict) and all(options.get(key) == value for key, value in EXPECTED_DEFAULT_PROFILE.items())


def validate_sample(sample: Any, invocation: BatchInvocation, expected_order: int) -> dict[str, Any]:
    if not isinstance(sample, dict):
        raise BenchmarkFailure("benchmark batch contains a malformed sample")
    if sample.get("round") != invocation.round_number or sample.get("order_index") != expected_order:
        raise BenchmarkFailure("benchmark sample lost round or order identity")
    if sample.get("seed") != effective_seed(invocation):
        raise BenchmarkFailure("benchmark sample recorded an inconsistent seed")
    if sample.get("hard_gates_passed") is not True or sample.get("fixture_loopback_only") is not True:
        raise BenchmarkFailure("benchmark correctness hard gate failed")
    if sample.get("candidate") not in CANDIDATES:
        raise BenchmarkFailure("benchmark returned an unexpected candidate")
    if sample.get("phase") not in {"warmup", "measured", "safety"}:
        raise BenchmarkFailure("benchmark sample phase is invalid")
    if sample.get("scenario") in RANKED_SCENARIOS:
        if sample.get("profile") != "scaled-production-default-streaming":
            raise BenchmarkFailure("ranked sample used an invalid transfer profile")
        fixture = sample.get("fixture")
        metrics = sample.get("metrics")
        if not isinstance(fixture, dict) or fixture.get("streaming_body") is not True:
            raise BenchmarkFailure("ranked sample lacks streaming fixture evidence")
        if not isinstance(metrics, dict) or not isinstance(metrics.get("scenario_elapsed_ns"), int):
            raise BenchmarkFailure("ranked sample lacks in-process timing")
        if sample.get("file_integrity_passed") is not True or sample.get("no_leak") is not True:
            raise BenchmarkFailure("ranked sample failed integrity or cleanup")
    elif sample.get("scenario") not in SAFETY_SCENARIOS:
        raise BenchmarkFailure("benchmark returned an unexpected scenario")
    return sample


def validate_payload(payload: Any, invocation: BatchInvocation) -> list[dict[str, Any]]:
    if not isinstance(payload, dict) or payload.get("schema_version") != SCHEMA_VERSION:
        raise BenchmarkFailure("benchmark produced an invalid batch schema")
    if payload.get("fixture_loopback_only") is not True or payload.get("hard_gates_passed") is not True:
        raise BenchmarkFailure("benchmark batch correctness hard gate failed")
    if not profile_is_production_default(payload.get("profile")):
        raise BenchmarkFailure("benchmark batch did not use production transfer defaults")
    round_value = payload.get("round")
    if not isinstance(round_value, dict) or round_value.get("number") != invocation.round_number:
        raise BenchmarkFailure("benchmark batch round identity is invalid")
    if round_value.get("seed") != effective_seed(invocation):
        raise BenchmarkFailure("benchmark batch recorded an inconsistent seed")
    if round_value.get("order_policy") != "seeded-fisher-yates-by-phase":
        raise BenchmarkFailure("benchmark batch order policy is invalid")
    samples = payload.get("samples")
    expected_performance = len(CANDIDATES) * len(RANKED_SCENARIOS) * (
        invocation.warmups + invocation.measured_repetitions
    )
    expected_total = expected_performance + len(SAFETY_SCENARIOS)
    if not isinstance(samples, list) or len(samples) != expected_total:
        raise BenchmarkFailure("benchmark batch sample count is invalid")
    validated = [validate_sample(sample, invocation, index) for index, sample in enumerate(samples)]
    counts = Counter(
        (sample["scenario"], sample["candidate"], sample["phase"])
        for sample in validated
        if sample["scenario"] in RANKED_SCENARIOS
    )
    for scenario in RANKED_SCENARIOS:
        for candidate in CANDIDATES:
            if counts[(scenario, candidate, "warmup")] != invocation.warmups:
                raise BenchmarkFailure("benchmark batch lost a warmup sample")
            if counts[(scenario, candidate, "measured")] != invocation.measured_repetitions:
                raise BenchmarkFailure("benchmark batch lost a measured sample")
    safety_names = {sample["scenario"] for sample in validated if sample["phase"] == "safety"}
    if safety_names != set(SAFETY_SCENARIOS):
        raise BenchmarkFailure("benchmark batch safety coverage is incomplete")
    ensure_safe_text(json.dumps(payload, ensure_ascii=False))
    return validated


def invoke_batch(binary: Path, root: Path, invocation: BatchInvocation) -> dict[str, Any]:
    command = [
        str(binary),
        "--mode",
        "scaled-batch",
        "--round",
        str(invocation.round_number),
        "--seed",
        str(invocation.seed),
        "--warmups",
        str(invocation.warmups),
        "--measured-repetitions",
        str(invocation.measured_repetitions),
        "--schema-version",
        SCHEMA_VERSION,
    ]
    started = time.perf_counter_ns()
    cpu_time: int | str = "unavailable"
    peak_memory: int | str = "unavailable"
    with (
        tempfile.TemporaryFile() as stdout_file,
        tempfile.TemporaryFile() as stderr_file,
    ):
        process = subprocess.Popen(
            command,
            cwd=root,
            stdout=stdout_file,
            stderr=stderr_file,
        )
        while process.poll() is None:
            observed_cpu, observed_memory = windows_process_metrics(process.pid)
            if observed_cpu != "unavailable":
                cpu_time = observed_cpu
            if observed_memory != "unavailable":
                peak_memory = max(peak_memory if isinstance(peak_memory, int) else 0, observed_memory)
            time.sleep(0.05)
        process.wait(timeout=10)
        stdout_file.seek(0)
        stderr_file.seek(0)
        stdout = stdout_file.read().decode("utf-8", errors="replace")
        stderr = stderr_file.read().decode("utf-8", errors="replace")
    process_elapsed_ms = (time.perf_counter_ns() - started) // 1_000_000
    if process.returncode != 0:
        raise BenchmarkFailure("direct scaled benchmark batch exited non-zero")
    try:
        payload = json.loads(stdout)
    except json.JSONDecodeError as error:
        raise BenchmarkFailure("direct scaled benchmark emitted malformed JSON") from error
    samples = validate_payload(payload, invocation)
    return {
        "round": invocation.round_number,
        "command": [
            "admission_benchmark",
            "--mode",
            "scaled-batch",
            "--round",
            str(invocation.round_number),
            "--seed",
            str(invocation.seed),
            "--warmups",
            str(invocation.warmups),
            "--measured-repetitions",
            str(invocation.measured_repetitions),
            "--schema-version",
            SCHEMA_VERSION,
        ],
        "exit_code": process.returncode,
        "batch_process_elapsed_ms": int(process_elapsed_ms),
        "batch_process_cpu_time_ms": cpu_time,
        "batch_process_peak_resident_memory_bytes": peak_memory,
        "stderr": sanitize_stderr(stderr),
        "payload": payload,
        "samples": samples,
    }


def flatten_samples(batches: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [
        {
            "round": batch["round"],
            "batch_process_elapsed_ms": batch["batch_process_elapsed_ms"],
            "batch_process_cpu_time_ms": batch["batch_process_cpu_time_ms"],
            "batch_process_peak_resident_memory_bytes": batch[
                "batch_process_peak_resident_memory_bytes"
            ],
            "sample": sample,
        }
        for batch in batches
        for sample in batch["samples"]
    ]


def measured_records(records: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [
        record
        for record in records
        if record["sample"]["phase"] == "measured"
        and record["sample"]["scenario"] in RANKED_SCENARIOS
    ]


def aggregates_for(records: list[dict[str, Any]]) -> list[dict[str, Any]]:
    grouped: dict[tuple[str, str], list[dict[str, Any]]] = defaultdict(list)
    for record in records:
        sample = record["sample"]
        grouped[(sample["scenario"], sample["candidate"])].append(record)
    aggregates: list[dict[str, Any]] = []
    for (scenario, candidate), group in sorted(grouped.items()):
        metrics: dict[str, Any] = {}
        for metric in ("scenario_elapsed_ns", "effective_throughput_bytes_per_sec", "retry_count"):
            values = [numeric(record["sample"]["metrics"].get(metric)) for record in group]
            clean = [value for value in values if value is not None]
            metrics[metric] = robust_summary(clean)
        elapsed = metrics["scenario_elapsed_ns"]
        metrics["relative_mad"] = (
            elapsed["median_absolute_deviation"] / elapsed["median"]
            if elapsed["median"]
            else float("inf")
        )
        aggregates.append(
            {
                "scenario": scenario,
                "candidate": candidate,
                "samples": len(group),
                "metrics": metrics,
            }
        )
    compatibility = {
        entry["scenario"]: entry["metrics"]["effective_throughput_bytes_per_sec"]["median"]
        for entry in aggregates
        if entry["candidate"] == "compatibility"
    }
    for entry in aggregates:
        baseline = compatibility.get(entry["scenario"])
        throughput = entry["metrics"]["effective_throughput_bytes_per_sec"]["median"]
        entry["metrics"]["normalized_throughput_vs_compatibility"] = (
            throughput / baseline if baseline else 0.0
        )
    return aggregates


def round_aggregates(records: list[dict[str, Any]]) -> list[dict[str, Any]]:
    by_round: dict[int, list[dict[str, Any]]] = defaultdict(list)
    for record in records:
        by_round[record["round"]].append(record)
    rows: list[dict[str, Any]] = []
    for round_number, round_records in sorted(by_round.items()):
        for aggregate in aggregates_for(round_records):
            rows.append({"round": round_number, **aggregate})
    return rows


def aggregate_index(aggregates: list[dict[str, Any]]) -> dict[tuple[str, str], dict[str, Any]]:
    return {(entry["scenario"], entry["candidate"]): entry for entry in aggregates}


def duration_validity(aggregates: list[dict[str, Any]]) -> list[dict[str, Any]]:
    index = aggregate_index(aggregates)
    rows: list[dict[str, Any]] = []
    for scenario in RANKED_SCENARIOS:
        entry = index.get((scenario, "compatibility"))
        if entry is None:
            raise BenchmarkFailure("compatibility duration evidence is incomplete")
        median = entry["metrics"]["scenario_elapsed_ns"]["median"]
        rows.append(
            {
                "scenario": scenario,
                "compatibility_median_scenario_elapsed_ns": median,
                "minimum_ns": MIN_SCENARIO_DURATION_NS,
                "valid": median >= MIN_SCENARIO_DURATION_NS,
            }
        )
    return rows


def candidate_limits(candidate: str) -> tuple[int, int]:
    if candidate == "compatibility":
        return (1_000_000, 1_000_000)
    global_text, host_text = candidate.removeprefix("g").split("-h", 1)
    return (int(global_text), int(host_text))


def objectives_from_aggregates(aggregates: list[dict[str, Any]]) -> dict[str, dict[str, float]]:
    index = aggregate_index(aggregates)
    objectives: dict[str, dict[str, float]] = {}
    for candidate in CANDIDATES:
        pressure = index.get((PRESSURE_SCENARIO, candidate))
        non_pressure = [index.get((scenario, candidate)) for scenario in NON_PRESSURE_SCENARIOS]
        if pressure is None or any(entry is None for entry in non_pressure):
            continue
        global_limit, host_limit = candidate_limits(candidate)
        objectives[candidate] = {
            "pressure_retry_count": float(pressure["metrics"]["retry_count"]["median"]),
            "worst_normalized_non_pressure_throughput": min(
                float(entry["metrics"]["normalized_throughput_vs_compatibility"])
                for entry in non_pressure
                if entry is not None
            ),
            "global_limit": float(global_limit),
            "per_origin_limit": float(host_limit),
        }
    return objectives


def dominance_by_internal_elapsed(aggregates: list[dict[str, Any]]) -> dict[str, list[str]]:
    index = aggregate_index(aggregates)
    dominated: dict[str, list[str]] = {}
    for candidate in CANDIDATES:
        values = {
            scenario: index.get((scenario, candidate), {})
            .get("metrics", {})
            .get("scenario_elapsed_ns", {})
            .get("median")
            for scenario in RANKED_SCENARIOS
        }
        if not all(isinstance(value, (int, float)) for value in values.values()):
            continue
        dominators: list[str] = []
        for other in CANDIDATES:
            if other == candidate:
                continue
            other_values = {
                scenario: index.get((scenario, other), {})
                .get("metrics", {})
                .get("scenario_elapsed_ns", {})
                .get("median")
                for scenario in RANKED_SCENARIOS
            }
            if not all(isinstance(value, (int, float)) for value in other_values.values()):
                continue
            no_worse = all(other_values[scenario] <= values[scenario] for scenario in RANKED_SCENARIOS)
            strictly_better = any(
                other_values[scenario] < values[scenario] for scenario in RANKED_SCENARIOS
            )
            if no_worse and strictly_better:
                dominators.append(other)
        if dominators:
            dominated[candidate] = sorted(dominators)
    return dominated


def rank_candidates(objectives: dict[str, dict[str, float]]) -> list[str]:
    return sorted(
        objectives,
        key=lambda candidate: (
            objectives[candidate]["pressure_retry_count"],
            -objectives[candidate]["worst_normalized_non_pressure_throughput"],
            objectives[candidate]["global_limit"],
            objectives[candidate]["per_origin_limit"],
            candidate,
        ),
    )


def pareto_frontier(objectives: dict[str, dict[str, float]]) -> list[str]:
    frontier: list[str] = []
    for candidate, value in objectives.items():
        dominated = False
        for other, other_value in objectives.items():
            if other == candidate:
                continue
            no_worse = (
                other_value["pressure_retry_count"] <= value["pressure_retry_count"]
                and other_value["worst_normalized_non_pressure_throughput"]
                >= value["worst_normalized_non_pressure_throughput"]
            )
            strictly_better = (
                other_value["pressure_retry_count"] < value["pressure_retry_count"]
                or other_value["worst_normalized_non_pressure_throughput"]
                > value["worst_normalized_non_pressure_throughput"]
            )
            if no_worse and strictly_better:
                dominated = True
                break
        if not dominated:
            frontier.append(candidate)
    return sorted(frontier)


def round_stability(round_aggregate_rows: list[dict[str, Any]]) -> dict[str, Any]:
    by_round: dict[int, list[dict[str, Any]]] = defaultdict(list)
    for row in round_aggregate_rows:
        by_round[row["round"]].append(row)
    rankings: dict[str, list[str]] = {}
    frontiers: dict[str, list[str]] = {}
    positions: dict[str, list[int]] = defaultdict(list)
    memberships: dict[str, list[bool]] = defaultdict(list)
    for round_number, rows in sorted(by_round.items()):
        objectives = objectives_from_aggregates(rows)
        ranking = rank_candidates(objectives)
        frontier = pareto_frontier(objectives)
        rankings[str(round_number)] = ranking
        frontiers[str(round_number)] = frontier
        for candidate in CANDIDATES:
            positions[candidate].append(ranking.index(candidate) if candidate in ranking else len(CANDIDATES))
            memberships[candidate].append(candidate in frontier)
    candidates: dict[str, Any] = {}
    for candidate in CANDIDATES:
        rank_values = positions[candidate]
        membership_values = memberships[candidate]
        candidates[candidate] = {
            "round_ranks": rank_values,
            "rank_delta": max(rank_values) - min(rank_values) if rank_values else len(CANDIDATES),
            "frontier_membership": membership_values,
            "frontier_consistent": len(set(membership_values)) == 1,
            "stable": (
                bool(rank_values)
                and max(rank_values) - min(rank_values) <= 1
                and len(set(membership_values)) == 1
            ),
        }
    return {"rankings": rankings, "frontiers": frontiers, "candidates": candidates}


def eligibility(
    aggregates: list[dict[str, Any]],
    duration_rows: list[dict[str, Any]],
    stability: dict[str, Any],
) -> tuple[list[dict[str, Any]], str | None]:
    objectives = objectives_from_aggregates(aggregates)
    index = aggregate_index(aggregates)
    compatibility_pressure = objectives.get("compatibility", {}).get("pressure_retry_count")
    duration_valid = all(row["valid"] for row in duration_rows)
    rows: list[dict[str, Any]] = []
    eligible_candidates: list[str] = []
    for candidate in CANDIDATES:
        candidate_rows = [index.get((scenario, candidate)) for scenario in RANKED_SCENARIOS]
        variation_exceptions = sum(
            entry is not None and entry["metrics"]["relative_mad"] > 0.10
            for entry in candidate_rows
        )
        objective = objectives.get(candidate)
        pressure_ok = bool(
            objective is not None
            and compatibility_pressure is not None
            and objective["pressure_retry_count"] <= compatibility_pressure / 2
        )
        throughput_ok = bool(
            objective is not None
            and objective["worst_normalized_non_pressure_throughput"] >= 0.85
        )
        stable = stability["candidates"].get(candidate, {}).get("stable") is True
        row = {
            "candidate": candidate,
            "correctness": True,
            "duration_validity": duration_valid,
            "relative_mad_exceptions": variation_exceptions,
            "relative_mad_ok": variation_exceptions <= 1,
            "round_stable": stable,
            "pressure_retry_ok": pressure_ok,
            "non_pressure_throughput_ok": throughput_ok,
            "pressure_retry_median": objective["pressure_retry_count"] if objective else None,
            "worst_normalized_non_pressure_throughput": (
                objective["worst_normalized_non_pressure_throughput"] if objective else None
            ),
        }
        row["eligible"] = all(
            (
                row["correctness"],
                row["duration_validity"],
                row["relative_mad_ok"],
                row["round_stable"],
                row["pressure_retry_ok"],
                row["non_pressure_throughput_ok"],
            )
        )
        rows.append(row)
        if row["eligible"]:
            eligible_candidates.append(candidate)
    if not eligible_candidates:
        return rows, None
    ranking = rank_candidates({candidate: objectives[candidate] for candidate in eligible_candidates})
    return rows, ranking[0]


def correctness_table(records: list[dict[str, Any]]) -> list[dict[str, Any]]:
    grouped: dict[tuple[str, str], list[dict[str, Any]]] = defaultdict(list)
    for record in records:
        sample = record["sample"]
        grouped[(sample["scenario"], sample["candidate"])].append(record)
    rows = []
    for (scenario, candidate), group in sorted(grouped.items()):
        rows.append(
            {
                "scenario": scenario,
                "candidate": candidate,
                "samples": len(group),
                "passed": sum(record["sample"]["hard_gates_passed"] is True for record in group),
                "failed": sum(record["sample"]["hard_gates_passed"] is not True for record in group),
                "phases": sorted({record["sample"]["phase"] for record in group}),
            }
        )
    return rows


def numeric_record_summary(records: list[dict[str, Any]], key: str) -> dict[str, int | float] | None:
    values = [record[key] for record in records if numeric(record.get(key)) is not None]
    return robust_summary(values) if values else None


def tool_version(command: list[str], root: Path) -> str:
    return run_text(command, root).splitlines()[0]


def metadata(root: Path, branch: str, commit: str) -> dict[str, Any]:
    return {
        "timestamp_utc": datetime.now(UTC).isoformat().replace("+00:00", "Z"),
        "commit": commit,
        "branch": branch,
        "os": platform.platform(),
        "architecture": platform.machine()
        or os.environ.get("PROCESSOR_ARCHITECTURE")
        or os.environ.get("PROCESSOR_ARCHITEW6432")
        or sysconfig.get_platform()
        or "unavailable",
        "logical_cpu_count": os.cpu_count(),
        "rust_version": tool_version(["rustc", "--version"], root),
        "cargo_version": tool_version(["cargo", "--version"], root),
        "python_version": sys.version.split()[0],
        "build_profile": "release",
    }


def markdown_report(evidence: dict[str, Any]) -> str:
    protocol = evidence["protocol"]
    duration_rows = evidence["duration_validity"]
    eligibility_rows = evidence["eligibility"]
    process_cpu = numeric_record_summary(evidence["batches"], "batch_process_cpu_time_ms")
    process_memory = numeric_record_summary(
        evidence["batches"], "batch_process_peak_resident_memory_bytes"
    )
    process_elapsed = numeric_record_summary(evidence["batches"], "batch_process_elapsed_ms")
    lines = [
        "# D2B1-R scaled download-admission benchmark evidence",
        "",
        "## Scope",
        "",
        "- The fixture used local loopback listeners only; it made no external-network connection.",
        "- Production remained uncapped Compatibility; finite values were test-only candidates.",
        "- This evidence is calibration for a later D2B2 architecture review. It does not activate a finite policy, segmentation, aria2, or D3.",
        "",
        "## Machine and batch protocol",
        "",
        f"- Commit: {evidence['metadata']['commit']} on {evidence['metadata']['branch']}.",
        f"- OS / architecture: {evidence['metadata']['os']} / {evidence['metadata']['architecture']}.",
        f"- Toolchains: {evidence['metadata']['rust_version']}; {evidence['metadata']['cargo_version']}; Python {evidence['metadata']['python_version']}.",
        f"- Schema: {evidence['schema_version']}; batch rounds: {protocol['rounds']}; warmups per round: {protocol['warmups']}; measured samples per round: {protocol['measured_repetitions']}.",
        f"- Candidate order used a recorded fixed seed with independent round mixing: {protocol['base_seed']}.",
        "- Ranking and dominance use only in-process monotonic scenario_elapsed_ns. Process elapsed, CPU, and RSS are diagnostic-only.",
        "",
        "## Scaled fixture definitions",
        "",
    ]
    for scenario in RANKED_SCENARIOS:
        definition = SCENARIO_DEFINITIONS[scenario]
        lines.append(
            f"- {scenario}: origins={definition['origins']}; files={definition['files']}; total bytes={definition['total_bytes']}; server chunk bytes={definition['server_chunk_bytes']}; delays={definition['delay_ms']}."
        )
    lines.extend(
        [
            "- queue_soak is correctness-only and records cleanup, cancellation, pause/resume, fairness, and the ten-worker restart thread model.",
            "",
            "## Correctness",
            "",
            f"- Raw samples: {evidence['hard_gate_summary']['total']}; passed: {evidence['hard_gate_summary']['passed']}; failed: {evidence['hard_gate_summary']['failed']}.",
            "- Every retained batch/sample passed loopback, integrity, no-leak, no-replay, policy-bound, and applicable safety hard gates.",
            "",
            "| Scenario | Candidate | Samples | Passed | Failed | Phases |",
            "| --- | --- | ---: | ---: | ---: | --- |",
        ]
    )
    for row in evidence["correctness_by_scenario_candidate"]:
        lines.append(
            f"| {row['scenario']} | {row['candidate']} | {row['samples']} | {row['passed']} | {row['failed']} | {', '.join(row['phases'])} |"
        )
    lines.extend(
        [
            "",
            "## Duration validity and measured distributions",
            "",
            "| Scenario | Compatibility median internal ns | Minimum ns | Valid |",
            "| --- | ---: | ---: | --- |",
        ]
    )
    for row in duration_rows:
        lines.append(
            f"| {row['scenario']} | {row['compatibility_median_scenario_elapsed_ns']} | {row['minimum_ns']} | {row['valid']} |"
        )
    lines.extend(
        [
            "",
            "| Scenario | Candidate | Samples | Median internal ns | Relative MAD | Median throughput B/s | Normalized throughput | Median retries |",
            "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |",
        ]
    )
    for entry in evidence["aggregates"]:
        metrics = entry["metrics"]
        lines.append(
            "| {scenario} | {candidate} | {samples} | {elapsed} | {relative_mad:.4f} | {throughput} | {normalized:.4f} | {retry} |".format(
                scenario=entry["scenario"],
                candidate=entry["candidate"],
                samples=entry["samples"],
                elapsed=metrics["scenario_elapsed_ns"]["median"],
                relative_mad=metrics["relative_mad"],
                throughput=metrics["effective_throughput_bytes_per_sec"]["median"],
                normalized=metrics["normalized_throughput_vs_compatibility"],
                retry=metrics["retry_count"]["median"],
            )
        )
    lines.extend(
        [
            "",
            "## Round stability, dominance, and eligibility",
            "",
            f"- Per-round rankings: {json.dumps(evidence['round_stability']['rankings'], sort_keys=True)}.",
            f"- Per-round Pareto frontiers: {json.dumps(evidence['round_stability']['frontiers'], sort_keys=True)}.",
            "- A candidate is round-stable only when its two rank positions differ by at most one and its frontier membership is unchanged.",
            "- Dominance uses only median internal scenario duration, never process startup or runner elapsed.",
        ]
    )
    if evidence["dominance_by_internal_elapsed"]:
        for candidate, dominators in evidence["dominance_by_internal_elapsed"].items():
            lines.append(f"- {candidate} is dominated by {', '.join(dominators)} under internal-time dominance.")
    else:
        lines.append("- No candidate is strictly dominated across every ranked scenario under internal-time dominance.")
    lines.extend(
        [
            "",
            "| Candidate | Duration valid | MAD exceptions | Round stable | Pressure retry OK | Throughput OK | Eligible |",
            "| --- | --- | ---: | --- | --- | --- | --- |",
        ]
    )
    for row in eligibility_rows:
        lines.append(
            f"| {row['candidate']} | {row['duration_validity']} | {row['relative_mad_exceptions']} | {row['round_stable']} | {row['pressure_retry_ok']} | {row['non_pressure_throughput_ok']} | {row['eligible']} |"
        )
    recommendation = evidence["recommendation"]
    if recommendation is None:
        lines.append("- Result: no candidate satisfies every predeclared criterion; D2B1-R declines a D2B2 recommendation.")
    else:
        lines.append(
            f"- Result: {recommendation} is the sole evidence recommendation for a later D2B2 architecture review. This is not activation."
        )
    lines.extend(
        [
            "",
            "## Diagnostics and limitations",
            "",
            f"- Batch process elapsed range: {process_elapsed if process_elapsed is not None else 'unavailable'}.",
            f"- Batch process CPU range: {process_cpu if process_cpu is not None else 'unavailable'}.",
            f"- Batch process peak RSS range: {process_memory if process_memory is not None else 'unavailable'}.",
            "- Process diagnostics are intentionally excluded from ranking and dominance.",
            "- Evidence is loopback-only on one hardware profile. It does not establish live-host behavior, durable scheduler/Store restart behavior, or target-origin admission after redirects.",
        ]
    )
    return "\n".join(lines) + "\n"


def ensure_output_paths(args: argparse.Namespace) -> None:
    for path in (args.output_json, args.output_markdown):
        if path.exists() and not args.replace:
            raise BenchmarkFailure("an evidence output already exists; use --replace to replace it")
        path.parent.mkdir(parents=True, exist_ok=True)


def main() -> int:
    args = parse_args()
    root = args.repo_root.resolve()
    ensure_output_paths(args)
    verify_context(root, args.expected_branch, args.expected_commit)
    binary = build_example(root)
    batches = [
        invoke_batch(
            binary,
            root,
            BatchInvocation(round_number, args.seed, args.warmups, args.repetitions),
        )
        for round_number in range(1, args.rounds + 1)
    ]
    records = flatten_samples(batches)
    measured = measured_records(records)
    aggregates = aggregates_for(measured)
    duration_rows = duration_validity(aggregates)
    if not all(row["valid"] for row in duration_rows):
        raise BenchmarkFailure("scaled Compatibility duration validity rule failed")
    round_rows = round_aggregates(measured)
    stability = round_stability(round_rows)
    eligibility_rows, recommendation = eligibility(aggregates, duration_rows, stability)
    evidence = {
        "schema_version": SCHEMA_VERSION,
        "metadata": metadata(root, args.expected_branch, args.expected_commit),
        "protocol": {
            "build_command": ["cargo", "build", "--offline", "--release", "-p", "insar-download", "--example", "admission_benchmark"],
            "execution_mode": "build-once-two-batch-processes",
            "rounds": args.rounds,
            "warmups": args.warmups,
            "measured_repetitions": args.repetitions,
            "base_seed": args.seed,
            "candidates": CANDIDATES,
            "ranked_scenarios": RANKED_SCENARIOS,
            "safety_scenarios": SAFETY_SCENARIOS,
            "ranking_metric": "scenario_elapsed_ns",
            "process_metrics_ranked": False,
            "fixture_network": "loopback-only",
            "scenario_definitions": SCENARIO_DEFINITIONS,
        },
        "batches": batches,
        "samples": records,
        "aggregates": aggregates,
        "round_aggregates": round_rows,
        "duration_validity": duration_rows,
        "correctness_by_scenario_candidate": correctness_table(records),
        "dominance_by_internal_elapsed": dominance_by_internal_elapsed(aggregates),
        "round_stability": stability,
        "eligibility": eligibility_rows,
        "recommendation": recommendation,
        "hard_gate_summary": {
            "total": len(records),
            "passed": len(records),
            "failed": 0,
        },
    }
    report = markdown_report(evidence)
    encoded = json.dumps(evidence, ensure_ascii=False, sort_keys=True)
    ensure_safe_text(encoded + report)
    args.output_json.write_text(
        json.dumps(evidence, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    args.output_markdown.write_text(report, encoding="utf-8")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except BenchmarkFailure as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(1) from error
