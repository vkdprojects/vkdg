use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::sync::Arc;
use tokio::runtime::Runtime;

// ── helpers ───────────────────────────────────────────────────────────────────

fn make_rt() -> Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn make_conversation_op(n_messages: usize) -> vkdg_operations::Operation {
    use vkdg_operations::*;
    let messages = (0..n_messages)
        .map(|i| Message {
            role: if i % 2 == 0 { Role::User } else { Role::Assistant },
            content: MessageContent::Text(format!("{} ", "message ".repeat(20))),
        })
        .collect();
    Operation::Conversation(ConversationRequest {
        messages,
        tools: vec![],
        max_tokens: Some(100),
        temperature: None,
        stream: false,
        system: None,
        required_capabilities: CapabilitySet::default(),
    })
}

// ── Scenario 1: Admission + routing (no upstream) ────────────────────────────
// Measures pure gateway overhead: admission semaphore + route selection.
// Expected: < 5µs per call on modern hardware (in-process, no I/O).
fn bench_admission_routing(c: &mut Criterion) {
    use vkdg_connections::{AuthKind, ConnectionCatalog, ConnectionConfig, ProviderKind};
    use vkdg_core::{ApiType, ClientId, ConnectionId, RequestEnvelope, RequestId, TenantId};
    use vkdg_operations::CapabilitySet;
    use vkdg_routing::{EligibilityFilter, PluginHooks, RouteConfig, RouteId, Router, StrategyKind};

    let rt = make_rt();

    let conn_id = ConnectionId("bench-conn".into());
    let _catalog = Arc::new(ConnectionCatalog::new(vec![ConnectionConfig {
        id: conn_id.clone(),
        provider: ProviderKind::Anthropic,
        auth: AuthKind::ApiKey { env_var: "BENCH_KEY".into() },
        models: vec!["claude-*".into()],
        max_concurrent: 10_000,
        weight: 1,
        tags: vec![],
        capabilities: CapabilitySet::default(),
    }]));

    let route = RouteConfig {
        id: RouteId("bench".into()),
        match_models: vec!["claude-*".into()],
        strategy: StrategyKind::RoundRobin,
        targets: vec![conn_id],
        plugin_hooks: PluginHooks::default(),
    };
    let router = Arc::new(Router::new(vec![route]));

    let envelope = RequestEnvelope {
        request_id: RequestId::new(),
        client_id: ClientId("bench".into()),
        tenant_id: TenantId("default".into()),
        session_key: None,
        api_type: ApiType::AnthropicMessages,
        model_requested: "claude-3-5-haiku-20241022".into(),
        deadline: None,
    };

    c.bench_function("admission_routing", |b| {
        b.to_async(&rt).iter(|| async {
            let result = router
                .route(&envelope, &EligibilityFilter::default())
                .await;
            criterion::black_box(result)
        })
    });
}

// ── Scenario 2: SSE parse throughput ─────────────────────────────────────────
// Measures how fast the SSE parser processes a stream of N chunks.
// Expected: > 100 MB/s throughput (allocation-light byte scanning).
fn bench_sse_parsing(c: &mut Criterion) {
    use vkdg_http::sse::SseParser;

    // Pre-build SSE event chunks — one complete event per chunk.
    let chunk: Vec<u8> =
        b"data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"hello world benchmark\"}}\n\n"
            .to_vec();
    let chunks: Vec<Vec<u8>> = (0..100).map(|_| chunk.clone()).collect();

    let mut group = c.benchmark_group("sse_parse");
    for n in [10usize, 100].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(n), n, |b, &n| {
            b.iter(|| {
                let mut parser = SseParser::new();
                let mut total = 0usize;
                for ch in &chunks[..n] {
                    total += parser.push(ch).len();
                }
                criterion::black_box(total)
            })
        });
    }
    group.finish();
}

// ── Scenario 3: AnthropicAdapter prepare() throughput ────────────────────────
// Measures provider adapter serialization: Operation → PreparedRequest.
// Expected: < 10µs per call even for 10-message conversations.
fn bench_provider_prepare(c: &mut Criterion) {
    use vkdg_connections::{AuthKind, ConnectionConfig, ProviderKind};
    use vkdg_core::ConnectionId;
    use vkdg_http::provider::ProviderAdapter;
    use vkdg_operations::CapabilitySet;
    use vkdg_provider_anthropic::AnthropicAdapter;

    let config = ConnectionConfig {
        id: ConnectionId("bench".into()),
        provider: ProviderKind::Anthropic,
        auth: AuthKind::ApiKey { env_var: "BENCH_KEY".into() },
        models: vec!["claude-3-5-haiku-20241022".into()],
        max_concurrent: 1_000,
        weight: 1,
        tags: vec![],
        capabilities: CapabilitySet::default(),
    };
    let adapter = AnthropicAdapter;
    let token = "sk-bench-token";

    let mut group = c.benchmark_group("provider_prepare");
    for n_messages in [1usize, 10, 50].iter() {
        let op = make_conversation_op(*n_messages);
        group.bench_with_input(
            BenchmarkId::from_parameter(n_messages),
            n_messages,
            |b, _| {
                b.iter(|| criterion::black_box(adapter.prepare(&op, &config, token)))
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_admission_routing, bench_sse_parsing, bench_provider_prepare);
criterion_main!(benches);
