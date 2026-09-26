# VKDG Benchmarks

Measured on: Apple M-series, macOS, rustc 1.98.1
Date: 2026-09-26

All benchmarks use warm in-process state (no network I/O).
These measure gateway overhead, not provider latency.

## Scenario 1: Admission + routing overhead

Simulates: request arrives, admission semaphore acquired, routing decision made.
No upstream call. Measures pure gateway overhead.

| Metric | Value |
|---|---|
| mean | 71.1 ns |
| 95% CI | [71.0 ns – 71.2 ns] |

## Scenario 2: SSE parser throughput

Simulates: N chunks of a streaming response parsed. Each chunk is a complete
`data: {...}\n\n` event (~79 bytes).

| Chunk count | Time | Throughput |
|---|---|---|
| 10 chunks | 1.662 µs | ~475 MB/s |
| 100 chunks | 16.24 µs | ~486 MB/s |

## Scenario 3: Provider adapter prepare() throughput

Simulates: Operation → PreparedRequest serialization (Anthropic adapter).

| Message count | Mean | 95% CI |
|---|---|---|
| 1 message | 652 ns | [636 ns – 683 ns] |
| 10 messages | 2.415 µs | [2.408 µs – 2.423 µs] |
| 50 messages | 10.06 µs | [10.03 µs – 10.09 µs] |

## Interpretation

Admission + routing at ~71 ns/request means the gateway adds well under 1 µs of
overhead before the first byte goes upstream. SSE parsing sustains ~480 MB/s,
sufficient for any realistic streaming response. Provider serialization scales
linearly at ~200 ns per additional message. The bottleneck is always
network/provider latency (typically 200–2000 ms), not the gateway.
