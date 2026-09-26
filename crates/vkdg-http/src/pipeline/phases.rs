use std::sync::Arc;

use vkdg_combos::BudgetPolicy;
use vkdg_core::{ConnectionId, RequestEnvelope};
use vkdg_memory::inject_memories;
use vkdg_operations::{MessageContent, Operation, Role};

use crate::PipelineState;

// ── Pipeline phase helpers ────────────────────────────────────────────────────

pub(super) struct ComboSessionResolution {
    /// Combo target list; non-empty = combo matched with target overrides.
    pub(super) resolved_targets: Option<Vec<ConnectionId>>,
    /// Combo id for debug tracing.
    pub(super) combo_id: Option<String>,
    pub(super) effective_compressor_id: Option<String>,
    pub(super) compression_threshold: u32,
    pub(super) session_preferred: Option<ConnectionId>,
    /// Budget policy from matched combo, if any.
    pub(super) budget_policy: Option<BudgetPolicy>,
}

pub(super) async fn resolve_combo_and_session(
    pipeline: &PipelineState,
    envelope: &RequestEnvelope,
) -> ComboSessionResolution {
    let combo = pipeline
        .combo_resolver
        .as_ref()
        .and_then(|r| r.resolve(&envelope.model_requested));

    if let Some(c) = &combo {
        tracing::debug!(combo_id = %c.id, "request resolved to combo");
    }

    let combo_compression = combo.as_ref().and_then(|c| c.compression.as_ref()).cloned();
    let budget_policy = combo.as_ref().and_then(|c| c.budget.as_ref()).cloned();
    let effective_compressor_id: Option<String> = envelope
        .compression_override
        .clone()
        .or_else(|| combo_compression.as_ref().map(|c| c.plugin_id.clone()));
    let compression_threshold: u32 = combo_compression
        .as_ref()
        .and_then(|c| c.auto_trigger_tokens)
        .unwrap_or(2000);

    let session_preferred = if let (Some(registry), Some(session_key)) =
        (&pipeline.session_registry, &envelope.session_key)
    {
        registry.get(&session_key.0).await
    } else {
        None
    };

    ComboSessionResolution {
        resolved_targets: combo.as_ref().map(|c| c.targets.clone()),
        combo_id: combo.map(|c| c.id.clone()),
        effective_compressor_id,
        compression_threshold,
        session_preferred,
        budget_policy,
    }
}

pub(super) async fn prepare_operation(
    pipeline: &PipelineState,
    mut operation: Operation,
    envelope: &RequestEnvelope,
    compression_threshold: u32,
    effective_compressor_id: &Option<String>,
) -> Operation {
    // Context compression (pre-dispatch)
    let skip_compression = effective_compressor_id.as_deref() == Some("none");
    if !skip_compression {
        if let Some(compressor) = &pipeline.compressor {
            if let Operation::Conversation(conv_req) = &operation {
                let estimated = compressor.estimate_tokens(conv_req);
                if estimated >= compression_threshold {
                    if let Some(comp_id) = effective_compressor_id {
                        if comp_id != "auto" && comp_id != compressor.name() {
                            tracing::debug!(
                                requested = %comp_id,
                                available = %compressor.name(),
                                "compression plugin mismatch, using available (plugin registry is Phase E)",
                            );
                        }
                    }
                    match compressor.compress(conv_req.clone(), estimated) {
                        Ok((compressed_req, report)) => {
                            tracing::debug!(
                                tokens_before = estimated,
                                tokens_after  = estimated.saturating_sub(report.estimated_tokens_removed),
                                algorithm     = %report.strategy,
                                "compression applied",
                            );
                            operation = Operation::Conversation(compressed_req);
                        }
                        Err(vkdg_policy_compress::CompressionError::NotApplicable) => {}
                        Err(e) => {
                            tracing::warn!(error = %e, "compression failed, proceeding uncompressed");
                        }
                    }
                }
            }
        }
    }

    // Global system prompt injection
    if let (Some(global_prompt), Operation::Conversation(conv_req)) =
        (&pipeline.global_system_prompt, &mut operation)
    {
        conv_req.system = Some(match &conv_req.system {
            None => global_prompt.clone(),
            Some(existing) => format!("{global_prompt}\n\n{existing}"),
        });
    }

    // Memory injection
    if let (Some(memory_store), Operation::Conversation(conv_req)) =
        (&pipeline.memory_store, &mut operation)
    {
        let query = conv_req
            .messages
            .last()
            .and_then(|m| match &m.content {
                MessageContent::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .unwrap_or("");
        let memories = memory_store.retrieve(&envelope.tenant_id.0, query, 5).await;
        if !memories.is_empty() {
            *conv_req = inject_memories(conv_req.clone(), &memories);
            tracing::debug!(count = memories.len(), "memories injected");
        }
    }

    operation
}

pub(super) fn post_response_accounting(
    pipeline: &PipelineState,
    ctx: &vkdg_core::pipeline::PipelineCtx,
    response_body: Option<&bytes::Bytes>,
    start_time: std::time::Instant,
    operation: &Operation,
) {
    // Memory extraction: non-streaming only (fire-and-forget)
    if response_body.is_some() {
        if let (Some(memory_store), Operation::Conversation(conv_req)) =
            (&pipeline.memory_store, operation)
        {
            let facts = vkdg_memory::extract_facts(&conv_req.messages);
            if !facts.is_empty() {
                let tenant = ctx.envelope.tenant_id.0.clone();
                let session = ctx.envelope.session_key.as_ref().map(|s| s.0.clone());
                let memory_store = Arc::clone(memory_store);
                tokio::spawn(async move {
                    for fact in facts {
                        memory_store
                            .store(vkdg_memory::MemoryRecord {
                                id: uuid::Uuid::new_v4(),
                                tenant_id: tenant.clone(),
                                session_id: session.clone(),
                                fact,
                                source: "conversation".into(),
                                created_at: chrono::Utc::now(),
                                expires_at: Some(chrono::Utc::now() + chrono::Duration::days(30)),
                                tags: vec![],
                            })
                            .await;
                    }
                });
            }
        }
    }

    // Eval scoring: non-streaming only
    if let Some(body) = response_body {
        if pipeline.eval_enabled {
            let metrics = vkdg_eval::LatencyMetrics {
                latency_ms: start_time.elapsed().as_millis() as u32,
                ttft_ms: None, // Phase E: track TTFT in streaming
                token_count: (body.len() / 4) as u32,
            };
            let eval = vkdg_eval::EvalScorer::score(
                &ctx.envelope.request_id.0.to_string(),
                ctx.connection_id
                    .as_ref()
                    .map(|c| c.0.as_str())
                    .unwrap_or(""),
                &String::from_utf8_lossy(body),
                metrics,
            );
            tracing::debug!(
                score = eval.score,
                status = ?eval.status,
                latency_ms = eval.latency_ms,
                "eval score",
            );
        }
    }

    // Quota + session pin: non-streaming only
    if let Some(approx_tokens) = response_body.map(|b| (b.len() / 4) as u64) {
        if let Some(tracker) = &pipeline.quota_tracker {
            if let Some(conn_id) = &ctx.connection_id {
                let tracker = Arc::clone(tracker);
                let conn_id = conn_id.clone();
                tokio::spawn(async move {
                    tracker.record_usage(&conn_id, approx_tokens).await;
                });
            }
        }
        if let (Some(registry), Some(session_key), Some(conn_id)) = (
            &pipeline.session_registry,
            &ctx.envelope.session_key,
            &ctx.connection_id,
        ) {
            let registry = Arc::clone(registry);
            let session_id = session_key.0.clone();
            let conn_id = conn_id.clone();
            tokio::spawn(async move {
                registry.pin(session_id, conn_id).await;
            });
        }
    }

    // Latency recording (unconditional)
    if let (Some(tracker), Some(conn_id)) = (&pipeline.latency_tracker, &ctx.connection_id) {
        let latency_ms = start_time.elapsed().as_millis() as u32;
        let tracker = Arc::clone(tracker);
        let conn_id = conn_id.clone();
        tokio::spawn(async move {
            tracker.record(&conn_id, latency_ms).await;
        });
    }

    // Cooldown recovery: clear expired cooldown on success → recover to Healthy
    if let Some(conn_arc) = ctx
        .connection_id
        .as_ref()
        .and_then(|id| pipeline.catalog.get(id))
    {
        if let Ok(mut conn) = conn_arc.try_write() {
            conn.check_cooldown();
        }
    }
}

/// Inject conversation context into the operation when a session rotates to a
/// different connection.  Called when the pipeline detects that the actual
/// connection differs from the pinned session preference.
///
/// Prepends a `[Account rotation: A -> B. Recent context:]` block to the system
/// prompt so the new provider has the last N turns of conversation history.
/// Non-conversation operations are left unchanged.
pub fn relay_on_rotation(
    operation: &mut Operation,
    rotated_from: &ConnectionId,
    rotated_to: &ConnectionId,
) {
    let Operation::Conversation(conv_req) = operation else {
        return;
    };

    // Preserve last 4 messages (≈2 turns) as context; earlier history is less
    // relevant and keeping it short avoids blowing up the system prompt.
    let recent: Vec<String> = conv_req
        .messages
        .iter()
        .rev()
        .take(4)
        .rev()
        .map(|m| {
            let role = match m.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => "system",
                Role::Tool => "tool",
            };
            let content = match &m.content {
                MessageContent::Text(t) => t.chars().take(200).collect::<String>(),
                _ => "[non-text content]".into(),
            };
            format!("{role}: {content}")
        })
        .collect();

    if recent.is_empty() {
        return;
    }

    let relay_block = format!(
        "[Account rotation: {} -> {}. Recent context:]\n{}",
        rotated_from.0,
        rotated_to.0,
        recent.join("\n")
    );

    tracing::info!(
        from = %rotated_from.0,
        to   = %rotated_to.0,
        messages = recent.len(),
        "context-relay on account rotation",
    );

    conv_req.system = Some(match &conv_req.system {
        None => relay_block,
        Some(existing) => format!("{relay_block}\n\n{existing}"),
    });
}
