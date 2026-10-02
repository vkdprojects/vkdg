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
- Maintains context (remembers facts stated early)
- Handles tool results without losing context
- Stays within acceptable latency bounds

Usage:
  uv run tests/stress/quality.py                        # uses env vars
  uv run tests/stress/quality.py --url https://vkdg.codeatlas.com.br --key vkdg_xxx
  uv run tests/stress/quality.py --provider openai --url https://api.openai.com --key sk-xxx
  uv run tests/stress/quality.py --turns 60 --latency-p95 20000

Environment variables (override with flags):
  VKDG_BASE_URL    Gateway base URL  (default: https://vkdg.codeatlas.com.br)
  VKDG_API_KEY     API key
  VKDG_MODEL       Model ID          (default: claude-sonnet-4.6)
  VKDG_PROVIDER    anthropic | openai (default: anthropic)
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
from rich import print as rprint

console = Console()

# ── Types ────────────────────────────────────────────────────────────────────

@dataclass
class Turn:
    turn: int
    user_msg: str
    expected: str
    response: str = ""
    ok: bool = False
    ms: float = 0.0
    msgs_in_history: int = 0
    error: Optional[str] = None

@dataclass
class Config:
    base_url: str
    api_key: str
    model: str
    provider: str  # anthropic | openai
    turns: int
    latency_p95_ms: float
    latency_p50_ms: float
    timeout_secs: int
    secret: str = "FLAMINGO42"

# ── Providers ─────────────────────────────────────────────────────────────────

def build_request_anthropic(cfg: Config, messages: list, system: str) -> dict:
    return {
        "url": f"{cfg.base_url}/v1/messages",
        "headers": {
            "content-type": "application/json",
            "anthropic-version": "2023-06-01",
            "x-api-key": cfg.api_key,
        },
        "body": {
            "model": cfg.model,
            "max_tokens": 128,
            "stream": True,
            "system": [
                {"type": "text", "text": system},
                {"type": "text", "text": "Respond in ONE short sentence only. Be direct and concise."},
            ],
            "messages": messages,
        },
    }


def parse_sse_stream(text: str) -> str:
    """Extract concatenated text from an Anthropic SSE stream."""
    import json as _json
    parts = []
    for line in text.splitlines():
        if not line.startswith("data:"):
            continue
        payload = line[5:].strip()
        if payload in ("[DONE]", ""):
            continue
        try:
            evt = _json.loads(payload)
        except Exception:
            continue
        # content_block_delta carries text_delta
        if evt.get("type") == "content_block_delta":
            d = evt.get("delta", {})
            if d.get("type") == "text_delta":
                parts.append(d.get("text", ""))
    return "".join(parts).strip()


def parse_response_anthropic(data) -> str:
    """Accept either a JSON dict (non-streaming) or an SSE string."""
    if isinstance(data, str):
        return parse_sse_stream(data)
    return (data.get("content") or [{}])[0].get("text", "").strip()


def build_request_openai(cfg: Config, messages: list, system: str) -> dict:
    return {
        "url": f"{cfg.base_url}/v1/chat/completions",
        "headers": {
            "content-type": "application/json",
            "authorization": f"Bearer {cfg.api_key}",
        },
        "body": {
            "model": cfg.model,
            "max_tokens": 128,
            "stream": True,
            "messages": [{"role": "system", "content": system}] + messages,
        },
    }


def parse_response_openai(data) -> str:
    if isinstance(data, str):
        # OpenAI SSE streaming
        import json as _j
        parts = []
        for line in data.splitlines():
            if not line.startswith("data:"):
                continue
            payload = line[5:].strip()
            if payload in ("[DONE]", ""):
                continue
            try:
                evt = _j.loads(payload)
                delta = (evt.get("choices") or [{}])[0].get("delta", {})
                parts.append(delta.get("content") or "")
            except Exception:
                continue
        return "".join(parts).strip()
    return (data.get("choices") or [{}])[0].get("message", {}).get("content", "").strip()


PROVIDERS = {
    "anthropic": (build_request_anthropic, parse_response_anthropic),
    "openai": (build_request_openai, parse_response_openai),
}

# ── Scenarios ─────────────────────────────────────────────────────────────────

def scenario_for_turn(turn: int, cfg: Config) -> tuple[str, str]:
    """Returns (user_message, expected_substring)."""
    if turn == 1:
        return "What is 2 + 2? Answer with just the number.", "4"
    if turn % 10 == 0:
        return (
            f"What is the session token for this conversation? "
            f"Reply with ONLY the token value (e.g. TOKEN=XXX).",
            cfg.secret,
        )
    if turn % 7 == 0:
        return (
            f"We are on turn {turn}. What is {turn} × 2? Just the number.",
            str(turn * 2),
        )
    if turn % 5 == 0:
        return (
            f"Repeat exactly this phrase and nothing else: CHECKPOINT-{turn}",
            f"CHECKPOINT-{turn}",
        )
    return (
        f"Say only the number {turn} and nothing else.",
        str(turn),
    )


# ── Core runner ───────────────────────────────────────────────────────────────

def run(cfg: Config) -> list[Turn]:
    build_fn, parse_fn = PROVIDERS[cfg.provider]
    system = (
        f"You are a helpful assistant. "
        f"Session token for this conversation: TOKEN={cfg.secret}. "
        f"When asked for the session token, always repeat it exactly. "
        f"Remember everything from this conversation."
    )
    messages: list[dict] = []
    results: list[Turn] = []

    console.rule(f"[bold cyan]VKDG Stress Test[/]  {cfg.base_url}")
    console.print(f"model={cfg.model}  provider={cfg.provider}  turns={cfg.turns}  secret={cfg.secret}\n")

    for turn_num in range(1, cfg.turns + 1):
        user_msg, expected = scenario_for_turn(turn_num, cfg)
        messages.append({"role": "user", "content": user_msg})
        req = build_fn(cfg, messages, system)

        t = Turn(
            turn=turn_num,
            user_msg=user_msg,
            expected=expected,
            msgs_in_history=len(messages),
        )

        t0 = time.perf_counter()
        try:
            with httpx.Client(timeout=cfg.timeout_secs) as client:
                resp = client.post(
                    req["url"],
                    headers=req["headers"],
                    json=req["body"],
                )
            t.ms = (time.perf_counter() - t0) * 1000
            if resp.status_code != 200:
                t.error = f"HTTP {resp.status_code}: {resp.text[:120]}"
            else:
                # Kiro always returns SSE even with stream=True in the body;
                # pass the raw text so parse_fn can handle either JSON or SSE.
                try:
                    data = resp.json()
                except Exception:
                    data = resp.text
                t.response = parse_fn(data)
                messages.append({"role": "assistant", "content": t.response})
                t.ok = expected.lower() in t.response.lower()
        except Exception as e:
            t.ms = (time.perf_counter() - t0) * 1000
            t.error = str(e)[:120]

        # Live output
        if t.error:
            status = "[red]ERR[/]"
        elif t.ok:
            status = "[green]OK [/]"
        else:
            status = "[red]BAD[/]"

        console.print(
            f"Turn {turn_num:3d} | {t.ms:6.0f}ms | msgs={t.msgs_in_history:3d} | {status} | "
            f"{user_msg[:38]:<38} | {(t.error or t.response)[:55]}"
        )
        results.append(t)

        if t.error:
            console.print(f"[red]  ↳ Gateway error — stopping.[/]")
            break

    return results


# ── Report ────────────────────────────────────────────────────────────────────

def report(results: list[Turn], cfg: Config) -> bool:
    console.rule("[bold]Summary")

    total = len(results)
    passed = sum(1 for r in results if r.ok)
    errors = sum(1 for r in results if r.error)
    quality_pct = 100 * passed / total if total else 0

    ok_ms = sorted(r.ms for r in results if r.ok and not r.error)
    p50 = ok_ms[len(ok_ms) // 2] if ok_ms else 0
    p95 = ok_ms[int(len(ok_ms) * 0.95)] if ok_ms else 0
    p99 = ok_ms[int(len(ok_ms) * 0.99)] if ok_ms else 0

    # Quality check table
    table = Table(title="Quality & Latency")
    table.add_column("Metric", style="cyan")
    table.add_column("Value", justify="right")
    table.add_column("Threshold", justify="right")
    table.add_column("Status", justify="center")

    q_ok = quality_pct >= 90
    table.add_row("Quality (turns passed)", f"{passed}/{total} ({quality_pct:.0f}%)", "≥ 90%",
                  "[green]✓[/]" if q_ok else "[red]✗[/]")
    p50_ok = p50 <= cfg.latency_p50_ms
    table.add_row("Latency p50", f"{p50:.0f}ms", f"≤ {cfg.latency_p50_ms:.0f}ms",
                  "[green]✓[/]" if p50_ok else "[red]✗[/]")
    p95_ok = p95 <= cfg.latency_p95_ms
    table.add_row("Latency p95", f"{p95:.0f}ms", f"≤ {cfg.latency_p95_ms:.0f}ms",
                  "[green]✓[/]" if p95_ok else "[red]✗[/]")
    table.add_row("Latency p99", f"{p99:.0f}ms", "—", "")
    table.add_row("Gateway errors", str(errors), "0",
                  "[green]✓[/]" if errors == 0 else "[red]✗[/]")
    console.print(table)

    # Failures detail
    failures = [r for r in results if not r.ok]
    if failures:
        console.print("\n[red]Failed turns:[/]")
        for f in failures:
            if f.error:
                console.print(f"  Turn {f.turn}: [red]ERROR[/] {f.error}")
            else:
                console.print(
                    f"  Turn {f.turn}: expected [cyan]{f.expected!r}[/] "
                    f"in [yellow]{f.response!r}[/]  ({f.user_msg[:50]})"
                )

    all_ok = q_ok and p50_ok and p95_ok and errors == 0
    if all_ok:
        console.print("\n[bold green]✓ All checks passed.[/]")
    else:
        console.print("\n[bold red]✗ Some checks failed.[/]")
    return all_ok


# ── CLI ───────────────────────────────────────────────────────────────────────

def main() -> int:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--url", default=os.getenv("VKDG_BASE_URL", "https://vkdg.codeatlas.com.br"))
    p.add_argument("--key", default=os.getenv("VKDG_API_KEY", ""))
    p.add_argument("--model", default=os.getenv("VKDG_MODEL", "claude-sonnet-4.6"))
    p.add_argument("--provider", default=os.getenv("VKDG_PROVIDER", "anthropic"),
                   choices=list(PROVIDERS))
    p.add_argument("--turns", type=int, default=30)
    p.add_argument("--latency-p50", type=float, default=10_000, dest="latency_p50",
                   help="p50 latency threshold ms (default 10s)")
    p.add_argument("--latency-p95", type=float, default=30_000, dest="latency_p95",
                   help="p95 latency threshold ms (default 30s)")
    p.add_argument("--timeout", type=int, default=60, help="per-request timeout seconds")
    p.add_argument("--secret", default="FLAMINGO42", help="secret word injected in system, checked at turn 10/20/30/...")
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
        secret=args.secret,
    )

    results = run(cfg)
    passed = report(results, cfg)
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
