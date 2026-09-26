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
    let (ctx, outcome, final_attempt) =
        if let Err(VkdgError::UpstreamError { code: 429, .. }) = &outcome {
            if ctx.can_retry() {
                // Emit DecisionRecord for the failed first attempt before retrying.
                let excluded: Vec<ConnectionId> = ctx.connection_id.clone().into_iter().collect();
                emit_decision_record(&pipeline, &ctx, &outcome, 1);

                let mut ctx2 = PipelineCtx::new(ctx.envelope.clone());
                let outcome2 =
                    run_pipeline_inner(&pipeline, &mut ctx2, operation.clone(), &excluded).await;
                (ctx2, outcome2, 2u32)
            } else {
                (ctx, outcome, 1u32)
            }
        } else {
            (ctx, outcome, 1u32)
        };

    // Emit DecisionRecord for the final attempt (1 on first-try success/failure, 2 after retry).
    emit_decision_record(&pipeline, &ctx, &outcome, final_attempt);

    match outcome {
        Ok(resp) => resp,
        Err(e) => error_response(e),
    }
}

/// Emit a DecisionRecord to the pipeline exporter.
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
        route_id: ctx
            .connection_id
            .as_ref()
            .map(|c| c.0.clone())
            .unwrap_or_else(|| "none".into()),
        connection_chosen: ctx.connection_id.clone(),
        candidates_excluded: vec![],
        attempt_count: attempt,
        state_transitions: ctx.transitions.clone(),
        result,
    };
    pipeline.exporter.export(&record);

    // Push to admin request log if wired.
    if let Some(log) = &pipeline.request_log {
        use vkdg_admin::handlers::requests::RequestRecord;
        use vkdg_core::ApiType;

        let status = match outcome {
            Ok(_) => "completed",
            Err(_) => "failed",
        }
        .to_string();

        // Derive started_at_ms from the first state transition (Received timestamp).
        let started_at_ms = ctx
            .transitions
            .first()
            .map(|(_, t)| t.timestamp_millis())
            .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
        let duration_ms = chrono::Utc::now().timestamp_millis() - started_at_ms;

        let api_type_str = match &ctx.envelope.api_type {
            ApiType::AnthropicMessages => "anthropic",
            ApiType::OpenAiChatCompletions => "openai",
            ApiType::OpenAiResponses => "openai-responses",
            ApiType::OpenAiImages => "openai-images",
            ApiType::VkdgNative => "vkdg",
        }
        .to_string();

        log.push(RequestRecord {
            request_id: ctx.envelope.request_id.0.to_string(),
            model: ctx.envelope.model_requested.clone(),
            api_type: api_type_str,
            status,
            connection_id: ctx.connection_id.as_ref().map(|c| c.0.clone()),
            started_at_ms,
            duration_ms: Some(duration_ms),
            decision: None,
        });
    }
}
