use std::sync::Arc;

use axum::response::Response;
use vkdg_core::pipeline::PipelineCtx;
use vkdg_core::{AttemptResult, ConnectionId, DecisionRecord, VkdgError};
use vkdg_operations::Operation;

use super::helpers::error_response;
use super::inner::run_pipeline_inner;
use crate::PipelineState;

pub async fn run_conversation_pipeline(
    pipeline: Arc<PipelineState>,
    mut ctx: PipelineCtx,
    operation: Operation,
) -> Response {
    let outcome = run_pipeline_inner(&pipeline, &mut ctx, operation.clone(), &[]).await;

    // Transparent 429 fallback: if the upstream rate-limits us and we have not
    // yet committed any bytes to the client, retry with the failed connection
    // excluded so the router picks a different candidate.
    let (ctx, outcome, final_attempt) = match &outcome {
        Err(VkdgError::UpstreamError {
            code: code @ (429 | 529),
            ..
        }) if ctx.can_retry() => {
            let code = *code;
            let excluded: Vec<ConnectionId> = ctx.connection_id.clone().into_iter().collect();
            emit_decision_record(&pipeline, &ctx, &outcome, 1);

            let mut ctx2 = PipelineCtx::new(ctx.envelope.clone());
            let outcome2 =
                run_pipeline_inner(&pipeline, &mut ctx2, operation.clone(), &excluded).await;
            // If the retry also failed, carry the original code for metrics.
            let _ = code; // suppress unused-var on non-debug builds
            (ctx2, outcome2, 2u32)
        }
        _ => (ctx, outcome, 1u32),
    };

    // Emit DecisionRecord for the final attempt (1 on first-try success/failure, 2 after retry).
    emit_decision_record(&pipeline, &ctx, &outcome, final_attempt);

    let pending = pending_log(&pipeline, &ctx, &outcome, final_attempt, &operation);
    let mut response = match outcome {
        Ok(resp) => resp,
        Err(e) => error_response(&e),
    };
    if let Some(pending) = pending {
        response.extensions_mut().insert(pending);
    }
    response
}

/// Emit a `DecisionRecord` to the pipeline exporter.
pub(super) fn emit_decision_record(
    pipeline: &PipelineState,
    ctx: &PipelineCtx,
    outcome: &Result<Response, VkdgError>,
    attempt: u32,
) {
    let result = match outcome {
        Ok(_) => AttemptResult::Completed,
        Err(e) => AttemptResult::Failed {
            code: format!("{e}"),
            phase: format!("{:?}", ctx.state),
            retryable: !ctx.committed,
            committed: ctx.committed,
        },
    };
    let record = DecisionRecord {
        request_id: ctx.envelope.request_id.clone(),
        config_version: 0,
        client_id: ctx.envelope.client_id.clone(),
        route_id: ctx.route_id.clone().unwrap_or_else(|| "none".into()),
        connection_chosen: ctx.connection_id.clone(),
        candidates_excluded: ctx.excluded.clone(),
        attempt_count: attempt,
        state_transitions: ctx.transitions.clone(),
        result,
    };
    pipeline.exporter.export(&record);
}

/// The request-history row for a pipeline response, handed to
/// [`crate::require_api_key`] in the response extensions. The middleware owns
/// the write: it knows the key's `no_log` and sees the body finish, so a row is
/// never written for a no-log key and a streamed row gets its tokens and final
/// status when the stream ends.
#[derive(Clone)]
pub struct PendingLog {
    pub log: Arc<vkdg_admin::handlers::requests::RequestLog>,
    pub record: vkdg_admin::handlers::requests::RequestRecord,
    /// List price of the model on the connection that served it, from its
    /// provider plugin. `None` = the provider declares no per-token price.
    pub price: Option<vkdg_core::pricing::ModelPrice>,
}

fn pending_log(
    pipeline: &PipelineState,
    ctx: &PipelineCtx,
    outcome: &Result<Response, VkdgError>,
    attempt: u32,
    operation: &Operation,
) -> Option<PendingLog> {
    use vkdg_admin::handlers::requests::{
        DecisionInfo, ExcludedInfo, RequestRecord, STATUS_PENDING,
    };
    use vkdg_core::ApiType;

    let log = pipeline.request_log.as_ref()?;
    let ok = outcome.is_ok();
    // A successful response is only headers so far; the body decides the rest.
    let status = if ok { STATUS_PENDING } else { "failed" }.to_string();
    // Derive started_at_ms from the first state transition (Received timestamp).
    let started_at_ms = ctx.transitions.first().map_or_else(
        || chrono::Utc::now().timestamp_millis(),
        |(_, t)| t.timestamp_millis(),
    );
    let duration_ms = chrono::Utc::now().timestamp_millis() - started_at_ms;
    let api_type = match &ctx.envelope.api_type {
        ApiType::AnthropicMessages => "anthropic",
        ApiType::OpenAiChatCompletions => "openai",
        ApiType::OpenAiResponses => "openai-responses",
        ApiType::OpenAiImages => "openai-images",
        ApiType::VkdgNative => "vkdg",
    }
    .to_string();

    let (thinking_requested, message_count) = match operation {
        Operation::Conversation(req) => (
            Some(req.thinking.is_some()),
            Some(u32::try_from(req.messages.len()).unwrap_or(u32::MAX)),
        ),
        _ => (None, None),
    };
    let error_message = if ok {
        None
    } else {
        outcome.as_ref().err().map(|e| e.to_string())
    };

    Some(PendingLog {
        price: if ok { ctx.price.clone() } else { None },
        log: Arc::clone(log),
        record: RequestRecord {
            request_id: ctx.envelope.request_id.0.to_string(),
            model: ctx.envelope.model_requested.clone(),
            api_type,
            status,
            connection_id: ctx.connection_id.as_ref().map(|c| c.0.clone()),
            key_id: Some(ctx.envelope.client_id.0.clone()),
            started_at_ms,
            duration_ms: Some(duration_ms),
            decision: ctx.route_id.as_ref().map(|route| DecisionInfo {
                route_id: Some(route.clone()),
                attempt_count: attempt,
                candidates_excluded: ctx
                    .excluded
                    .iter()
                    .map(|c| ExcludedInfo {
                        id: c.connection_id.0.clone(),
                        reason: c.reason.clone(),
                    })
                    .collect(),
            }),
            input_tokens: None,
            output_tokens: None,
            cost_microdollars: None,
            stop_reason: None,
            error_message,
            thinking_requested,
            message_count,
        },
    })
}
