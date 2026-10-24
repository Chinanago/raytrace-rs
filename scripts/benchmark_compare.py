"""Alternate owned-fixture Rust baselines and candidates; reject changed reports.

Build examples/throughput_check.rs at each revision, preserving the two executables.
This driver uses only the standard library. Windows memory figures cover the entire
benchmark process, including the materialized launch array and signature generation.
"""
from __future__ import annotations

import argparse
import ctypes
from ctypes import wintypes
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import time


def memory_reader():
    if os.name != "nt":
        return lambda process: None

    class Counters(ctypes.Structure):
        _fields_ = [("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD)] + [
            (name, ctypes.c_size_t)
            for name in [
                "PeakWorkingSetSize", "WorkingSetSize", "QuotaPeakPagedPoolUsage",
                "QuotaPagedPoolUsage", "QuotaPeakNonPagedPoolUsage",
                "QuotaNonPagedPoolUsage", "PagefileUsage", "PeakPagefileUsage", "PrivateUsage",
            ]
        ]

    query = ctypes.WinDLL("psapi", use_last_error=True).GetProcessMemoryInfo
    query.argtypes = [wintypes.HANDLE, ctypes.c_void_p, wintypes.DWORD]
    query.restype = wintypes.BOOL

    def read(process):
        counters = Counters()
        counters.cb = ctypes.sizeof(counters)
        if query(wintypes.HANDLE(int(process._handle)), ctypes.byref(counters), counters.cb):
            return counters.PeakWorkingSetSize, counters.PrivateUsage
        return None

    return read


def run(binary, args, output, read_memory):
    command = [str(binary), str(args.rays), str(args.threads), str(args.repeats), args.case, args.mode]
    peak_working = peak_private = 0
    started = time.perf_counter()
    with output.open("wb") as stdout, output.with_suffix(".log").open("wb") as stderr:
        process = subprocess.Popen(command, stdout=stdout, stderr=stderr)
        if args.affinity is not None:
            assign = ctypes.WinDLL("kernel32", use_last_error=True).SetProcessAffinityMask
            assign.argtypes = [wintypes.HANDLE, ctypes.c_size_t]
            assign.restype = wintypes.BOOL
            if not assign(wintypes.HANDLE(int(process._handle)), args.affinity):
                process.terminate()
                process.wait()
                raise ctypes.WinError(ctypes.get_last_error())
        while process.poll() is None:
            memory = read_memory(process)
            if memory is not None:
                peak_working = max(peak_working, memory[0])
                peak_private = max(peak_private, memory[1])
            time.sleep(0.025)
        if process.returncode:
            raise RuntimeError(f"failed ({process.returncode}): {command}; see {output.with_suffix('.log')}")
    data = json.loads(output.read_text(encoding="utf-8-sig"))
    return {
        "output": str(output), "wall_seconds_including_setup_and_checks": time.perf_counter() - started,
        "peak_working_set_bytes": peak_working or None,
        "peak_sampled_private_bytes": peak_private or None,
        "data": data,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--rays", type=int, default=1_048_576)
    parser.add_argument("--threads", type=int, default=4)
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--pairs", type=int, default=3)
    parser.add_argument("--case", default="all", choices=["all", "lens", "lens_200", "asphere", "array_256", "rod"])
    parser.add_argument("--mode", default="both", choices=["explicit", "streaming", "both"])
    parser.add_argument("--affinity", type=lambda value: int(value, 0), help="Optional Windows logical-CPU mask, e.g. 0x35")
    args = parser.parse_args()
    if min(args.rays, args.threads, args.repeats, args.pairs) < 1:
        parser.error("rays, threads, repeats and pairs must be positive")
    if args.affinity is not None and (os.name != "nt" or args.affinity < 1):
        parser.error("affinity requires Windows and a positive CPU mask")
    args.baseline, args.candidate = args.baseline.resolve(), args.candidate.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    read_memory = memory_reader()
    runs = []
    for pair in range(args.pairs):
        order = ["baseline", "candidate"] if pair % 2 == 0 else ["candidate", "baseline"]
        batch = {"pair": pair + 1, "order": order}
        for name in order:
            print(f"Pair {pair+1}/{args.pairs}: {name}, {args.rays} rays, {args.threads} threads", flush=True)
            destination = args.output / f"pair_{pair+1}_{name}.json"
            batch[name] = run(getattr(args, name), args, destination, read_memory)
        before, after = batch["baseline"]["data"]["cases"], batch["candidate"]["data"]["cases"]
        if len(before) != len(after):
            raise AssertionError("different fixture counts")
        for a, b in zip(before, after):
            for field in ["case", "rays", "surfaces", "threads", "source_signature", "report_signature", "report"]:
                if a[field] != b[field]:
                    raise AssertionError(f"changed {field} in {a['case']}; inspect {batch['baseline']['output']} and {batch['candidate']['output']}")
            print(f"  {a['case']}: reports identical; " + "; ".join(
                f"{field}: {a[field]:.6f}s -> {b[field]:.6f}s ({a[field]/b[field]:.3f}x)"
                for field in ["explicit_seconds", "streaming_seconds"] if a[field] is not None
            ), flush=True)
        runs.append(batch)
        (args.output / "progress.json").write_text(json.dumps(runs, indent=2), encoding="utf-8")
    cases = []
    for index, first in enumerate(runs[0]["baseline"]["data"]["cases"]):
        result = {"case": first["case"], "rays": first["rays"], "threads": first["threads"], "all_reports_identical": True}
        for field in ["explicit_seconds", "streaming_seconds"]:
            if first[field] is None:
                continue
            before = [pair["baseline"]["data"]["cases"][index][field] for pair in runs]
            after = [pair["candidate"]["data"]["cases"][index][field] for pair in runs]
            result[field] = {
                "baseline": statistics.median(before), "candidate": statistics.median(after),
                "speedup": statistics.median(before) / statistics.median(after),
                "baseline_process_medians": before, "candidate_process_medians": after,
            }
        cases.append(result)
    summary = {
        "platform": platform.platform(), "processor": platform.processor(),
        "baseline_sha256": hashlib.sha256(args.baseline.read_bytes()).hexdigest(),
        "candidate_sha256": hashlib.sha256(args.candidate.read_bytes()).hexdigest(),
        "pairs": args.pairs, "repeats_per_process": args.repeats,
        "logical_cpu_affinity": hex(args.affinity) if args.affinity is not None else None,
        "timing": runs[0]["baseline"]["data"]["timing"],
        "memory_scope": "Whole benchmark process, including materialized rays, input fingerprinting and untimed verification; Windows working-set high-water mark and sampled private bytes.",
        "memory_runs": [{name: {k: v for k, v in pair[name].items() if k != "data"} for name in ["baseline", "candidate"]} for pair in runs],
        "cases": cases,
    }
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
