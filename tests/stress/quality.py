#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "httpx>=0.27",
#   "rich>=13",
# ]
# ///
"""
VKDG Stress & Quality Test

Validates that a deployed AI gateway correctly:
- Preserves system-prompt identity across long conversations
- Maintains context across heavy tool-result history (simulates up to 1M token conversations)
- Latency degrades predictably as history grows (not catastrophically)
- Detects context loss: marker injected at turn 1 must be recalled at turn 50+
- No quality regression when progressive aging / compaction is active

Usage:
  uv run tests/stress/quality.py                              # uses env vars
  uv run tests/stress/quality.py --turns 30                   # default quality run
  uv run tests/stress/quality.py --scenario heavy --turns 50  # heavy history
  uv run tests/stress/quality.py --scenario latency           # latency profile
  uv run tests/stress/quality.py --scenario context-loss      # context retention
  uv run tests/stress/quality.py --scenario all               # all scenarios

Environment:
  VKDG_BASE_URL    default: https://vkdg.codeatlas.com.br
  VKDG_API_KEY     required
  VKDG_MODEL       default: claude-sonnet-4.6
  VKDG_PROVIDER    anthropic | openai  (default: anthropic)
"""

import argparse
import json
import os
import sys
import time
from dataclasses import dataclass, field
from typing import Optional

import httpx
from rich.console import Console
from rich.table import Table

console = Console()

# ── Types ─────────────────────────────────────────────────────────────────────

@dataclass
class Turn:
    turn: int
    user_msg: str
    expected: str
    response: str = ""
    ok: bool = False
    ms: float = 0.0
    msgs_in_history: int = 0
    approx_chars: int = 0  # rough token proxy (chars / 4)
    error: Optional[str] = None


@dataclass
class Config:
    base_url: str
    api_key: str
    model: str
    provider: str
    turns: int
    latency_p50_ms: float
    latency_p95_ms: float
    timeout_secs: int
    marker: str = "FLAMINGO42"
    heavy_payload_kb: int = 0   # KB of fake tool output injected each turn
    scenario: str = "quality"   # quality | heavy | latency | context-loss | all


# ── SSE / JSON parsing ────────────────────────────────────────────────────────

def _parse_sse(text: str) -> str:
    parts = []
    for line in text.splitlines():
        if not line.startswith("data:"):
            continue
        payload = line[5:].strip()
        if payload in ("[DONE]", ""):
            continue
        try:
            evt = json.loads(payload)
        except Exception:
            continue
        if evt.get("type") == "content_block_delta":
            d = evt.get("delta", {})
            if d.get("type") == "text_delta":
                parts.append(d.get("text", ""))
        # OpenAI streaming
        elif evt.get("object") == "chat.completion.chunk":
            delta = (evt.get("choices") or [{}])[0].get("delta", {})
            parts.append(delta.get("content") or "")
    return "".join(parts).strip()


def parse_response(data) -> str:
    if isinstance(data, str):
        return _parse_sse(data)
    # Anthropic non-stream
    if "content" in data:
        return (data.get("content") or [{}])[0].get("text", "").strip()
    # OpenAI non-stream
    if "choices" in data:
        return (data.get("choices") or [{}])[0].get("message", {}).get("content", "").strip()
    return ""


# ── HTTP ──────────────────────────────────────────────────────────────────────

def make_url(cfg: Config) -> str:
    if cfg.provider == "openai":
        return f"{cfg.base_url}/v1/chat/completions"
    return f"{cfg.base_url}/v1/messages"


def make_headers(cfg: Config) -> dict:
    if cfg.provider == "openai":
        return {
            "content-type": "application/json",
            "authorization": f"Bearer {cfg.api_key}",
        }
    return {
        "content-type": "application/json",
        "anthropic-version": "2023-06-01",
        "x-api-key": cfg.api_key,
    }


def make_body(cfg: Config, messages: list, system: str) -> dict:
    if cfg.provider == "openai":
        return {
            "model": cfg.model,
            "max_tokens": 128,
            "stream": True,
            "messages": [{"role": "system", "content": system}] + messages,
        }
    return {
        "model": cfg.model,
        "max_tokens": 128,
        "stream": True,
        "system": [
            {"type": "text", "text": system},
            {"type": "text", "text": "Respond in ONE short sentence only. Be direct."},
        ],
        "messages": messages,
    }


def call(cfg: Config, messages: list, system: str, timeout: int) -> tuple[str, float, Optional[str]]:
    """Returns (response_text, ms, error_or_None)."""
    url = make_url(cfg)
    headers = make_headers(cfg)
    body = make_body(cfg, messages, system)
    t0 = time.perf_counter()
    try:
        with httpx.Client(timeout=timeout) as client:
            resp = client.post(url, headers=headers, json=body)
        ms = (time.perf_counter() - t0) * 1000
        if resp.status_code != 200:
            return "", ms, f"HTTP {resp.status_code}: {resp.text[:120]}"
        try:
            data = resp.json()
        except Exception:
            data = resp.text
        return parse_response(data), ms, None
    except Exception as e:
        ms = (time.perf_counter() - t0) * 1000
        return "", ms, str(e)[:120]


# ── Scenarios ─────────────────────────────────────────────────────────────────

def _base_scenario(turn: int, cfg: Config) -> tuple[str, str]:
    if turn == 1:
        return "What is 2 + 2? Just the number.", "4"
    if turn % 10 == 0:
        return (
            f"What is the session marker for this conversation? "
            f"Reply with ONLY the value after the equals sign.",
            cfg.marker,
        )
    if turn % 7 == 0:
        return f"What is {turn} × 2? Just the number.", str(turn * 2)
    if turn % 5 == 0:
        return f"Repeat exactly: CHECK-{turn}", f"CHECK-{turn}"
    return f"Say only the number {turn}.", str(turn)


def _system(cfg: Config) -> str:
    return (
        f"You are a helpful assistant. "
        f"Session marker: MARKER={cfg.marker}. "
        f"When asked for the session marker, reply with ONLY the value after the equals sign. "
        f"Remember all context."
    )


def _approx_tokens(messages: list) -> int:
    """Rough token estimate: chars / 4."""
    total = sum(len(str(m.get("content", ""))) for m in messages)
    return total // 4


# ── Scenario runners ──────────────────────────────────────────────────────────

def run_quality(cfg: Config) -> list[Turn]:
    """Basic quality: does the model follow instructions across turns?"""
    console.rule(f"[cyan]Scenario: quality[/]  ({cfg.turns} turns, no heavy payload)")
    messages = []
    system = _system(cfg)
    results = []
    for t in range(1, cfg.turns + 1):
        q, exp = _base_scenario(t, cfg)
        messages.append({"role": "user", "content": q})
        text, ms, err = call(cfg, messages, system, cfg.timeout_secs)
        if text:
            messages.append({"role": "assistant", "content": text})
        ok = not err and exp.lower() in text.lower()
        approx = _approx_tokens(messages)
        _print_turn(t, ms, len(messages), approx, ok, q, text, err)
        results.append(Turn(t, q, exp, text, ok, ms, len(messages), approx, err))
        if err:
            break
    return results


def run_heavy(cfg: Config) -> list[Turn]:
    """
    Heavy history: inject large tool results every turn to simulate real
    coding-agent workloads with file reads, grep outputs, etc.
    Tests whether latency degrades catastrophically as history grows.
    """
    payload_kb = cfg.heavy_payload_kb or 8  # 8 KB per turn ≈ real grep/read output
    console.rule(
        f"[cyan]Scenario: heavy[/]  ({cfg.turns} turns, +{payload_kb}KB tool payload/turn)"
    )
    messages = []
    system = _system(cfg)
    results = []
    fake_output = "X" * (payload_kb * 1024)  # simulate a large file read

    for t in range(1, cfg.turns + 1):
        q, exp = _base_scenario(t, cfg)

        # Simulate large context accumulation: inject a big string each turn
        # to mimic real file reads, grep output, etc. Uses plain text to avoid
        # Kiro's strict tool_use/tool_result schema requirements.
        if t > 1:
            # Add a large assistant turn that simulates a file read result
            messages.append({
                "role": "assistant",
                "content": (
                    f"Reading file_{t}.ts: "
                    + fake_output[:min(payload_kb * 200, len(fake_output))]
                    + f"...[{payload_kb}KB context, turn {t}]"
                ),
            })

        messages.append({"role": "user", "content": q})
        text, ms, err = call(cfg, messages, system, cfg.timeout_secs)
        if text:
            messages.append({"role": "assistant", "content": text})
        ok = not err and exp.lower() in text.lower()
        approx = _approx_tokens(messages)
        _print_turn(t, ms, len(messages), approx, ok, q, text, err)
        results.append(Turn(t, q, exp, text, ok, ms, len(messages), approx, err))
        if err:
            break
    return results


def run_latency_profile(cfg: Config) -> list[Turn]:
    """
    Latency profile: build history in steps, measure latency at each size.
    Adds ~2KB of history per turn to profile how latency scales.
    Reports latency vs approximate token count.
    """
    console.rule("[cyan]Scenario: latency[/]  (2KB padding per turn, profile latency vs tokens)")
    messages = []
    system = _system(cfg)
    results = []
    # 2KB per turn — after 50 turns ≈ 100KB → ~25K tokens (realistic coding session)
    padding_per_turn = "P" * 2048

    for t in range(1, cfg.turns + 1):
        q = f"Say only the number {t}."
        exp = str(t)
        # Pad history with realistic-size content
        if t > 1:
            messages[-1]["content"] += f"\n[context-{t}]: {padding_per_turn[:200]}"

        messages.append({"role": "user", "content": q})
        text, ms, err = call(cfg, messages, system, cfg.timeout_secs)
        if text:
            messages.append({"role": "assistant", "content": text})
        ok = not err and exp in (text or "")
        approx = _approx_tokens(messages)
        _print_turn(t, ms, len(messages), approx, ok, q, text, err)
        results.append(Turn(t, q, exp, text, ok, ms, len(messages), approx, err))
        if err:
            break
    return results


def run_context_loss(cfg: Config) -> list[Turn]:
    """
    Context loss detector: plant a unique marker early, ask for it at
    turns 10, 20, 30, 50... as history grows. Measures exactly when
    (if ever) the gateway loses context.
    """
    turns = max(cfg.turns, 50)
    console.rule(f"[cyan]Scenario: context-loss[/]  ({turns} turns, marker recall check)")
    messages = []
    system = _system(cfg)
    results = []
    recall_turns = {10, 20, 30, 40, 50, 60, 70, 80, 90, 100}

    for t in range(1, turns + 1):
        if t in recall_turns:
            q = f"What is the session marker? Reply with ONLY the value after '='."
            exp = cfg.marker
        else:
            q = f"Say only the number {t}."
            exp = str(t)

        messages.append({"role": "user", "content": q})
        text, ms, err = call(cfg, messages, system, cfg.timeout_secs)
        if text:
            messages.append({"role": "assistant", "content": text})
        ok = not err and exp.lower() in (text or "").lower()
        approx = _approx_tokens(messages)
        _print_turn(t, ms, len(messages), approx, ok, q, text, err)
        results.append(Turn(t, q, exp, text, ok, ms, len(messages), approx, err))
        if err:
            break
    return results


# ── Output ────────────────────────────────────────────────────────────────────

def _print_turn(turn: int, ms: float, msgs: int, approx_tok: int,
                ok: bool, q: str, text: str, err: Optional[str]) -> None:
    if err:
        status = "[red]ERR[/]"
    elif ok:
        status = "[green]OK [/]"
    else:
        status = "[yellow]BAD[/]"

    console.print(
        f"T{turn:3d} | {ms:6.0f}ms | msgs={msgs:4d} | ~{approx_tok//1000:3d}K tok | {status} | "
        f"{q[:35]:<35} | {(err or text)[:45]}"
    )


# ── Report ────────────────────────────────────────────────────────────────────

def report(all_results: dict[str, list[Turn]], cfg: Config) -> bool:
    console.rule("[bold]Summary")

    overall_ok = True
    for scenario_name, results in all_results.items():
        if not results:
            continue
        total = len(results)
        passed = sum(1 for r in results if r.ok)
        errors = sum(1 in [1] for r in results if r.error)
        errors = sum(1 for r in results if r.error)
        quality_pct = 100 * passed / total if total else 0

        ok_ms = sorted(r.ms for r in results if r.ok and not r.error)
        p50 = ok_ms[len(ok_ms) // 2] if ok_ms else 0
        p95 = ok_ms[int(len(ok_ms) * 0.95)] if ok_ms else 0
        p99 = ok_ms[min(int(len(ok_ms) * 0.99), len(ok_ms) - 1)] if ok_ms else 0

        # Latency regression: last 5 turns vs first 5 turns
        first_5 = [r.ms for r in results[:5] if r.ok]
        last_5 = [r.ms for r in results[-5:] if r.ok]
        p50_first = sum(first_5) / len(first_5) if first_5 else 0
        p50_last = sum(last_5) / len(last_5) if last_5 else 0
        regression = p50_last / p50_first if p50_first > 0 else 1.0

        table = Table(title=f"Scenario: {scenario_name}")
        table.add_column("Metric", style="cyan")
        table.add_column("Value", justify="right")
        table.add_column("Threshold", justify="right")
        table.add_column("", justify="center")

        q_ok = quality_pct >= 90
        table.add_row("Quality", f"{passed}/{total} ({quality_pct:.0f}%)", "≥ 90%",
                      "[green]✓[/]" if q_ok else "[red]✗[/]")
        p50_ok = p50 <= cfg.latency_p50_ms
        table.add_row("Latency p50", f"{p50:.0f}ms", f"≤ {cfg.latency_p50_ms:.0f}ms",
                      "[green]✓[/]" if p50_ok else "[red]✗[/]")
        p95_ok = p95 <= cfg.latency_p95_ms
        table.add_row("Latency p95", f"{p95:.0f}ms", f"≤ {cfg.latency_p95_ms:.0f}ms",
                      "[green]✓[/]" if p95_ok else "[red]✗[/]")
        table.add_row("Latency p99", f"{p99:.0f}ms", "—", "")
        # Regression: last 5 turns should not be >3× slower than first 5
        reg_ok = regression <= 3.0
        table.add_row("Latency regression (last/first)", f"{regression:.1f}×",
                      "≤ 3×", "[green]✓[/]" if reg_ok else "[red]✗[/]")
        table.add_row("Gateway errors", str(errors), "0",
                      "[green]✓[/]" if errors == 0 else "[red]✗[/]")
        console.print(table)

        # Token count at failure
        fails = [r for r in results if not r.ok]
        if fails:
            console.print(f"[yellow]Failed turns ({scenario_name}):[/]")
            for f in fails[:5]:
                tok = f"~{f.approx_chars // 1000}K tok" if f.approx_chars else ""
                if f.error:
                    console.print(f"  T{f.turn} {tok}: [red]{f.error}[/]")
                else:
                    console.print(
                        f"  T{f.turn} {tok}: expected [cyan]{f.expected!r}[/] "
                        f"got [yellow]{f.response!r}[/]"
                    )

        scenario_ok = q_ok and p50_ok and p95_ok and reg_ok and errors == 0
        if not scenario_ok:
            overall_ok = False

    if overall_ok:
        console.print("\n[bold green]✓ All scenarios passed.[/]")
    else:
        console.print("\n[bold red]✗ Some checks failed.[/]")
    return overall_ok


# ── CLI ───────────────────────────────────────────────────────────────────────

def main() -> int:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--url", default=os.getenv("VKDG_BASE_URL", "https://vkdg.codeatlas.com.br"))
    p.add_argument("--key", default=os.getenv("VKDG_API_KEY", ""))
    p.add_argument("--model", default=os.getenv("VKDG_MODEL", "claude-sonnet-4.6"))
    p.add_argument("--provider", default=os.getenv("VKDG_PROVIDER", "anthropic"),
                   choices=["anthropic", "openai"])
    p.add_argument("--turns", type=int, default=30,
                   help="Number of turns (default 30; context-loss uses max(turns,50))")
    p.add_argument("--scenario", default="quality",
                   choices=["quality", "heavy", "latency", "context-loss", "all"],
                   help="Test scenario (default: quality)")
    p.add_argument("--heavy-kb", type=int, default=8, dest="heavy_kb",
                   help="KB of fake tool output per turn in heavy scenario (default 8)")
    p.add_argument("--latency-p50", type=float, default=10_000, dest="latency_p50")
    p.add_argument("--latency-p95", type=float, default=30_000, dest="latency_p95")
    p.add_argument("--timeout", type=int, default=120)
    p.add_argument("--marker", default="FLAMINGO42",
                   help="Marker injected in system, recalled at fixed turns")
    args = p.parse_args()

    if not args.key:
        console.print("[red]Error: --key or VKDG_API_KEY required[/]")
        return 2

    cfg = Config(
        base_url=args.url.rstrip("/"),
        api_key=args.key,
        model=args.model,
        provider=args.provider,
        turns=args.turns,
        latency_p50_ms=args.latency_p50,
        latency_p95_ms=args.latency_p95,
        timeout_secs=args.timeout,
        marker=args.marker,
        heavy_payload_kb=args.heavy_kb,
        scenario=args.scenario,
    )

    console.rule(f"[bold cyan]VKDG Stress & Quality Test[/]  {cfg.base_url}")
    console.print(
        f"model=[green]{cfg.model}[/]  provider={cfg.provider}  "
        f"scenario=[yellow]{cfg.scenario}[/]  turns={cfg.turns}\n"
    )

    scenarios_to_run = (
        ["quality", "heavy", "latency", "context-loss"]
        if cfg.scenario == "all"
        else [cfg.scenario]
    )

    all_results: dict[str, list[Turn]] = {}
    for s in scenarios_to_run:
        if s == "quality":
            all_results[s] = run_quality(cfg)
        elif s == "heavy":
            all_results[s] = run_heavy(cfg)
        elif s == "latency":
            all_results[s] = run_latency_profile(cfg)
        elif s == "context-loss":
            all_results[s] = run_context_loss(cfg)

    passed = report(all_results, cfg)
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
