//! Endpoint selection for Kiro / Amazon Q.
//!
//! Kiro's data plane is reachable through several hosts, and which one works
//! depends on how the account authenticates — not on preference:
//!
//! | kind     | host                                      | credential        |
//! |----------|-------------------------------------------|-------------------|
//! | `Ide`    | `runtime.{region}.kiro.dev`               | IDE/OAuth account |
//! | `ApiKey` | `codewhisperer.us-east-1.amazonaws.com`   | `ksk_` API key    |
//! |          | (`q.{region}.amazonaws.com` elsewhere)    |                   |
//!
//! Both route the operation in the URL path (`/generateAssistantResponse`) and
//! declare `origin: AI_EDITOR`. Three invariants come from measurements against
//! the live service, not from guesswork:
//!
//! 1. **API-key accounts must not send `profileArn`.** The key belongs to a
//!    tenant already; sending an ARN makes AWS answer 403.
//! 2. **IDE accounts belong on `runtime.*`.** That host answers the IDE protocol
//!    reliably, while `q.*` throttles it far harder. An API key on `runtime.*`
//!    is refused with "profileArn is required for this request."
//! 3. **An API key must stay on the editor protocol.** The same key reaches two
//!    different catalogues: posting to the service root with
//!    `x-amz-target: …GenerateAssistantResponse` and `origin: CLI` exposes only
//!    the legacy Amazon Q set (`claude-sonnet-4`, `claude-sonnet-4.5`,
//!    `claude-haiku-4.5`) and answers `INVALID_MODEL_ID` for everything else —
//!    including `auto`. Routing by path with `origin: AI_EDITOR` serves the full
//!    catalogue (20 ids: opus-5.5/5/4.8/4.7/4.6, sonnet-5/4.6, gpt-5.6-*, glm-5,
//!    minimax-*, qwen3-coder-next, deepseek-3.2, …), confirmed against
//!    `GET /ListAvailableModels?origin=AI_EDITOR` with the same credential.

use crate::auth::AUTH_API_KEY;
use crate::region::{is_valid_region, DEFAULT_REGION};

/// Regions where the Kiro planes (`runtime`, `management`, `telemetry`) exist.
/// Any other region has no `kiro.dev` host, so it must fall back.
pub const RUNTIME_REGIONS: [&str; 2] = ["us-east-1", "eu-central-1"];

/// Which host to use for a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointKind {
    /// Kiro IDE data plane, for OAuth accounts: `profileArn` in the body.
    Ide,
    /// CodeWhisperer editor plane, for `ksk_` API keys: no `profileArn`,
    /// `tokentype: API_KEY` header.
    ApiKey,
}

impl EndpointKind {
    /// Endpoint an account uses, derived from how it authenticates.
    ///
    /// API keys speak the CodeWhisperer plane; every OAuth account (Builder ID,
    /// IdC, social) belongs to the Kiro IDE data plane.
    pub fn for_auth_method(auth_method: &str) -> Self {
        if auth_method == AUTH_API_KEY {
            Self::ApiKey
        } else {
            Self::Ide
        }
    }

    /// Full request URL, falling back to the home region when this endpoint does
    /// not serve `region`.
    pub fn url(self, region: &str) -> String {
        let region = if self.serves_region(region) {
            region
        } else {
            DEFAULT_REGION
        };
        match self {
            Self::Ide => format!("https://runtime.{region}.kiro.dev/generateAssistantResponse"),
            // `codewhisperer.*` only resolves in the home region; every other
            // region serves the same editor protocol under `q.*`.
            Self::ApiKey if region == DEFAULT_REGION => {
                format!("https://codewhisperer.{region}.amazonaws.com/generateAssistantResponse")
            }
            Self::ApiKey => format!("https://q.{region}.amazonaws.com/generateAssistantResponse"),
        }
    }

    /// Whether the request body carries `profileArn`.
    ///
    /// API-key authentication does not accept it: AWS answers 403.
    pub fn sends_profile_arn(self) -> bool {
        matches!(self, Self::Ide)
    }

    /// Whether the request carries the `tokentype: API_KEY` header.
    pub fn sends_api_key_token_type(self) -> bool {
        matches!(self, Self::ApiKey)
    }

    /// `content-type` for the request body.
    ///
    /// Every plane speaks awsJson1_0, including `runtime.*`: third-party gateways
    /// running on the IDE plane send this header verbatim.
    pub fn content_type(self) -> &'static str {
        "application/x-amz-json-1.0"
    }

    /// True when `region` can serve this endpoint.
    ///
    /// The Kiro planes only exist in two regions; an unsupported region must fall
    /// back rather than produce a host that does not resolve.
    pub fn serves_region(self, region: &str) -> bool {
        match self {
            Self::Ide => RUNTIME_REGIONS.contains(&region),
            // The AWS hosts exist in more regions than the Kiro planes.
            Self::ApiKey => is_valid_region(region),
        }
    }

    /// `origin` the request declares, from the service's `Origin` enum.
    ///
    /// Both planes identify as an editor. `CLI` selects the legacy Amazon Q
    /// catalogue and must not be used (see the module docs).
    pub fn origin(self) -> &'static str {
        "AI_EDITOR"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{AUTH_BUILDER_ID, AUTH_IDC, AUTH_SOCIAL};

    // Refutes: sending profileArn on an API-key account, which AWS answers 403,
    // or omitting it for an IdC account, which then gets "not authorized".
    #[test]
    fn profile_arn_is_sent_only_on_the_ide_plane() {
        assert!(EndpointKind::for_auth_method(AUTH_IDC).sends_profile_arn());
        assert!(EndpointKind::for_auth_method(AUTH_BUILDER_ID).sends_profile_arn());
        assert!(EndpointKind::for_auth_method(AUTH_SOCIAL).sends_profile_arn());
        assert!(!EndpointKind::for_auth_method(AUTH_API_KEY).sends_profile_arn());
    }

    // Refutes: pointing OAuth accounts at the AWS hosts (heavily throttled for
    // the IDE protocol) or API keys at runtime.*, which answers
    // "profileArn is required for this request."
    #[test]
    fn each_auth_method_gets_its_own_host() {
        assert_eq!(
            EndpointKind::for_auth_method(AUTH_SOCIAL).url("us-east-1"),
            "https://runtime.us-east-1.kiro.dev/generateAssistantResponse"
        );
        assert_eq!(
            EndpointKind::for_auth_method(AUTH_API_KEY).url("us-east-1"),
            "https://codewhisperer.us-east-1.amazonaws.com/generateAssistantResponse"
        );
        assert_eq!(
            EndpointKind::for_auth_method(AUTH_API_KEY).url("eu-central-1"),
            "https://q.eu-central-1.amazonaws.com/generateAssistantResponse"
        );
    }

    // Refutes: keeping an API key on the legacy Amazon Q protocol (service root
    // + `x-amz-target` + `origin: CLI`). That plane answers INVALID_MODEL_ID for
    // claude-sonnet-5, glm-5, gpt-5.6-* and even `auto`, exposing only the three
    // legacy ids, while the same key serves all 20 models on the editor plane.
    #[test]
    fn an_api_key_stays_on_the_editor_protocol() {
        let api_key = EndpointKind::for_auth_method(AUTH_API_KEY);
        assert!(api_key
            .url("us-east-1")
            .ends_with("/generateAssistantResponse"));
        assert_eq!(api_key.origin(), "AI_EDITOR");
        assert!(api_key.sends_api_key_token_type());
    }

    // Refutes: routing by path and header at the same time, which no host accepts.
    #[test]
    fn operation_routing_is_always_in_the_path() {
        for kind in [EndpointKind::Ide, EndpointKind::ApiKey] {
            assert!(kind
                .url("us-east-1")
                .ends_with("/generateAssistantResponse"));
        }
    }

    // Refutes: templating a host for a region that has no kiro.dev DNS
    // (ap-southeast-1 is an inference region, never a client host).
    #[test]
    fn unsupported_regions_fall_back_to_the_home_region() {
        assert_eq!(
            EndpointKind::ApiKey.url("evil.example.com/"),
            "https://codewhisperer.us-east-1.amazonaws.com/generateAssistantResponse"
        );
        assert_eq!(
            EndpointKind::Ide.url(""),
            "https://runtime.us-east-1.kiro.dev/generateAssistantResponse"
        );
        assert_eq!(
            EndpointKind::Ide.url("ap-southeast-1"),
            "https://runtime.us-east-1.kiro.dev/generateAssistantResponse"
        );
        // eu-central-1 is one of the two real Kiro regions.
        assert_eq!(
            EndpointKind::Ide.url("eu-central-1"),
            "https://runtime.eu-central-1.kiro.dev/generateAssistantResponse"
        );
        // The AWS plane exists in more regions than the Kiro planes.
        assert!(EndpointKind::ApiKey.serves_region("us-west-2"));
        assert!(!EndpointKind::Ide.serves_region("us-west-2"));
    }

    // Refutes: sending plain JSON; every Kiro plane speaks awsJson1_0.
    #[test]
    fn every_plane_uses_aws_json_content_type() {
        for kind in [EndpointKind::Ide, EndpointKind::ApiKey] {
            assert_eq!(kind.content_type(), "application/x-amz-json-1.0");
        }
        assert!(!EndpointKind::Ide.sends_api_key_token_type());
        assert!(EndpointKind::ApiKey.sends_api_key_token_type());
    }
}
