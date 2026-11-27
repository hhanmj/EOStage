#!/usr/bin/env python3
"""SBAS-InSAR baseline network connectivity and closure phase RMS checker."""
from __future__ import annotations

import argparse
import collections
import json
import math
import sys
from typing import Any, Dict, List, Sequence, Tuple


def wrap_phase(rad: float) -> float:
    return (rad + math.pi) % (2.0 * math.pi) - math.pi


def closure_phase_rms(triplets: List[Tuple[float, float, float]]) -> float:
    if not triplets:
        return 0.0
    sq = [wrap_phase(p_ij + p_jk - p_ik) ** 2 for p_ij, p_jk, p_ik in triplets]
    return math.sqrt(sum(sq) / len(sq))


def analyze_baseline_network(
    pairs: Sequence[Tuple[str, str, float, float]],
    max_bperp_m: float = 200.0,
    max_btemp_days: float = 48.0,
) -> Dict[str, Any]:
    """Analyze SBAS interferometric network topology, components, and triangles.

    Each element of `pairs` is `(ref_epoch, sec_epoch, bperp_m, btemp_days)`.
    """
    adj: Dict[str, set[str]] = collections.defaultdict(set)
    admitted_edges: List[Dict[str, Any]] = []
    rejected_edges: List[Dict[str, Any]] = []

    for ref_epoch, sec_epoch, bperp_m, btemp_days in pairs:
        record = {
            "ref": str(ref_epoch),
            "sec": str(sec_epoch),
            "bperp_m": float(bperp_m),
            "btemp_days": float(btemp_days),
        }
        if abs(bperp_m) <= max_bperp_m and abs(btemp_days) <= max_btemp_days:
            admitted_edges.append(record)
            adj[str(ref_epoch)].add(str(sec_epoch))
            adj[str(sec_epoch)].add(str(ref_epoch))
        else:
            rejected_edges.append(record)

    nodes = sorted(adj.keys())
    visited: set[str] = set()
    components: List[List[str]] = []
    for node in nodes:
        if node in visited:
            continue
        comp: List[str] = []
        queue = collections.deque([node])
        visited.add(node)
        while queue:
            curr = queue.popleft()
            comp.append(curr)
            for nxt in sorted(adj[curr]):
                if nxt not in visited:
                    visited.add(nxt)
                    queue.append(nxt)
        components.append(sorted(comp))

    triangles: List[Tuple[str, str, str]] = []
    for i, u in enumerate(nodes):
        for j in range(i + 1, len(nodes)):
            v = nodes[j]
            if v not in adj[u]:
                continue
            for k in range(j + 1, len(nodes)):
                w = nodes[k]
                if w in adj[u] and w in adj[v]:
                    triangles.append((u, v, w))

    # Identify bridge edges whose removal increases connected component count
    bridges: List[Tuple[str, str]] = []
    seen_undirected: set[Tuple[str, str]] = set()
    for edge in admitted_edges:
        u, v = sorted((edge["ref"], edge["sec"]))
        if (u, v) in seen_undirected:
            continue
        seen_undirected.add((u, v))
        # BFS from u to v without edge (u, v)
        q = collections.deque([u])
        reached = {u}
        while q and v not in reached:
            curr = q.popleft()
            for nxt in adj[curr]:
                if (curr == u and nxt == v) or (curr == v and nxt == u):
                    continue
                if nxt not in reached:
                    reached.add(nxt)
                    q.append(nxt)
        if v not in reached:
            bridges.append((u, v))

    return {
        "epoch_count": len(nodes),
        "admitted_pair_count": len(admitted_edges),
        "rejected_pair_count": len(rejected_edges),
        "connected_component_count": len(components),
        "is_fully_connected": len(components) == 1 and len(nodes) > 0,
        "closed_triangle_count": len(triangles),
        "bridge_edges": [list(b) for b in bridges],
        "network_redundancy_ratio": round(
            len(triangles) / max(1, len(admitted_edges)), 4
        ),
    }


def detect_phase_unwrapping_errors(
    named_triplets: Sequence[Tuple[str, str, str, float, float, float]],
    cycle_threshold_rad: float = 1.5,
) -> Dict[str, Any]:
    """Evaluate closure phase residuals and integer 2*pi ambiguity jumps.

    Each element of `named_triplets` is `(t_i, t_j, t_k, phi_ij, phi_jk, phi_ik)`.
    """
    raw_triplets = [(p_ij, p_jk, p_ik) for _, _, _, p_ij, p_jk, p_ik in named_triplets]
    rms = closure_phase_rms(raw_triplets)
    edge_fault_counts: Dict[str, int] = collections.Counter()
    flagged_loops: List[Dict[str, Any]] = []

    for t_i, t_j, t_k, p_ij, p_jk, p_ik in named_triplets:
        raw_closure = p_ij + p_jk - p_ik
        wrapped_residual = wrap_phase(raw_closure)
        integer_cycles = int(round(raw_closure / (2.0 * math.pi)))
        if abs(wrapped_residual) >= cycle_threshold_rad or integer_cycles != 0:
            flagged_loops.append(
                {
                    "triplet": [t_i, t_j, t_k],
                    "wrapped_residual_rad": round(wrapped_residual, 6),
                    "integer_2pi_cycles": integer_cycles,
                }
            )
            edge_fault_counts[f"{t_i}_{t_j}"] += 1
            edge_fault_counts[f"{t_j}_{t_k}"] += 1
            edge_fault_counts[f"{t_i}_{t_k}"] += 1

    suspect_edges = [
        {"interferogram": edge, "faulty_loop_count": count}
        for edge, count in edge_fault_counts.most_common()
    ]
    return {
        "triplet_count": len(named_triplets),
        "closure_rms_rad": round(rms, 6),
        "flagged_loop_count": len(flagged_loops),
        "flagged_loops": flagged_loops,
        "suspect_interferograms": suspect_edges,
        "qc_passed": len(flagged_loops) == 0 and rms < cycle_threshold_rad,
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--input-json",
        help="Optional JSON string or path with 'triplets' and/or 'pairs'",
    )
    args = parser.parse_args(argv)

    if not args.input_json:
        sample = [(0.42, 0.31, 0.72), (1.15, -0.40, 0.76)]
        sample_pairs = [
            ("20240101", "20240113", 45.0, 12.0),
            ("20240113", "20240125", -30.0, 12.0),
            ("20240101", "20240125", 15.0, 24.0),
        ]
        net = analyze_baseline_network(sample_pairs)
        print(
            json.dumps(
                {
                    "triplet_count": len(sample),
                    "closure_rms_rad": round(closure_phase_rms(sample), 6),
                    "network_summary": net,
                }
            )
        )
        return 0

    payload = json.loads(args.input_json)
    result: Dict[str, Any] = {}
    if "named_triplets" in payload:
        result.update(
            detect_phase_unwrapping_errors(
                [tuple(item) for item in payload["named_triplets"]],
                float(payload.get("cycle_threshold_rad", 1.5)),
            )
        )
    elif "triplets" in payload:
        triplets = [tuple(item) for item in payload["triplets"]]
        result["triplet_count"] = len(triplets)
        result["closure_rms_rad"] = round(closure_phase_rms(triplets), 6)
    if "pairs" in payload:
        result["network_summary"] = analyze_baseline_network(
            [tuple(item) for item in payload["pairs"]],
            float(payload.get("max_bperp_m", 200.0)),
            float(payload.get("max_btemp_days", 48.0)),
        )
    print(json.dumps(result))
    return 0


if __name__ == "__main__":
    sys.exit(main())
