//! Endpoint selection for Kiro / Amazon Q.
//!
//! Kiro's data plane is reachable through several hosts, and which one works
//! depends on how the account authenticates — not on preference:
//!
//! | kind     | host                            | routing                                    | credential        |
//! |----------|---------------------------------|--------------------------------------------|-------------------|
//! | `Ide`    | `runtime.{region}.kiro.dev`     | operation in the URL path                  | IDE/OAuth account |
//! | `Cli`    | `q.{region}.amazonaws.com`      | `x-amz-target: …GenerateAssistantResponse` | `ksk_` API key    |
//! | `SendMessage` | `q.{region}.amazonaws.com` | `x-amz-target: …SendMessage`               | `ksk_` API key    |
//!
//! Two invariants come from third-party production measurements
//! (`dwgx/KiroStudio`, `src/kiro/endpoint/{ide,cli}.rs`), not from guesswork:
//!
//! 1. **API-key accounts must not send `profileArn`.** The key belongs to a
//!    tenant already; sending an ARN makes AWS answer 403.
//! 2. **IDE accounts belong on `runtime.*`.** That host answers the IDE protocol
//!    reliably, while `q.*` throttles it far harder.
//!
//! `SendMessage` exists as an escape hatch: it is a different operation bucket on
//! the same host, so it can still answer when `GenerateAssistantResponse` is
//! being throttled.

use crate::auth::AUTH_API_KEY;
use crate::region::{is_valid_region, DEFAULT_REGION};

/// Operation target for the CodeWhisperer streaming chat call.
pub const TARGET_GENERATE_ASSISTANT_RESPONSE: &str =
    "AmazonCodeWhispererStreamingService.GenerateAssistantResponse";

/// Operation target for the Amazon Q Developer streaming chat call.
pub const TARGET_SEND_MESSAGE: &str = "AmazonQDeveloperStreamingService.SendMessage";

/// Regions where the Kiro planes (`runtime`, `management`, `telemetry`) exist.
/// Any other region has no `kiro.dev` host, so it must fall back.
pub const RUNTIME_REGIONS: [&str; 2] = ["us-east-1", "eu-central-1"];

/// Which host and protocol shape to use for a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointKind {
    /// Kiro IDE data plane: operation in the path, `profileArn` in the body.
    Ide,
    /// Amazon Q CLI protocol: operation in `x-amz-target`, no `profileArn`.
    Cli,
    /// Same host as `Cli`, different operation bucket.
    SendMessage,
}

impl EndpointKind {
    /// Endpoint an account uses, derived from how it authenticates.
    ///
    /// API keys speak the CLI protocol; every OAuth account (Builder ID, IdC,
    /// social) belongs to the IDE data plane.
    pub fn for_auth_method(auth_method: &str) -> Self {
        if auth_method == AUTH_API_KEY {
            Self::Cli
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
            // The Kiro plane routes by path.
            Self::Ide => format!("https://runtime.{region}.kiro.dev/generateAssistantResponse"),
            // The legacy AWS plane posts to the service root and routes by header.
            Self::Cli | Self::SendMessage => format!("https://q.{region}.amazonaws.com/"),
        }
    }

    /// `x-amz-target` value, or `None` when the operation is in the URL.
    pub fn amz_target(self) -> Option<&'static str> {
        match self {
            Self::Ide => None,
            Self::Cli => Some(TARGET_GENERATE_ASSISTANT_RESPONSE),
            Self::SendMessage => Some(TARGET_SEND_MESSAGE),
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
        matches!(self, Self::Cli | Self::SendMessage)
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
            // The legacy AWS hosts exist in more regions than the Kiro planes.
            Self::Cli | Self::SendMessage => is_valid_region(region),
        }
    }

    /// `origin` the request declares, from the service's `Origin` enum.
    ///
    /// Plane and origin have to agree: the Kiro plane identifies as an editor,
    /// the Amazon Q plane as a CLI caller.
    pub fn origin(self) -> &'static str {
        match self {
            Self::Ide => "AI_EDITOR",
            Self::Cli | Self::SendMessage => "CLI",
        }
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

    // Refutes: pointing OAuth accounts at q.* (heavily throttled for the IDE
    // protocol) or API keys at runtime.* (403).
    #[test]
    fn each_auth_method_gets_its_own_host() {
        assert_eq!(
            EndpointKind::for_auth_method(AUTH_SOCIAL).url("us-east-1"),
            "https://runtime.us-east-1.kiro.dev/generateAssistantResponse"
        );
        assert_eq!(
            EndpointKind::for_auth_method(AUTH_API_KEY).url("eu-central-1"),
            "https://q.eu-central-1.amazonaws.com/"
        );
    }

    // Refutes: routing by path and header at the same time, which no host accepts.
    #[test]
    fn operation_routing_is_path_or_header_never_both() {
        assert_eq!(EndpointKind::Ide.amz_target(), None);
        assert!(EndpointKind::Ide
            .url("us-east-1")
            .ends_with("/generateAssistantResponse"));

        assert_eq!(
            EndpointKind::Cli.amz_target(),
            Some(TARGET_GENERATE_ASSISTANT_RESPONSE)
        );
        assert_eq!(
            EndpointKind::SendMessage.amz_target(),
            Some(TARGET_SEND_MESSAGE)
        );
        // The fallback bucket shares the host and differs only in the operation.
        assert_eq!(
            EndpointKind::SendMessage.url("us-east-1"),
            EndpointKind::Cli.url("us-east-1")
        );
    }

    // Refutes: templating a host for a region that has no kiro.dev DNS
    // (ap-southeast-1 is an inference region, never a client host).
    #[test]
    fn unsupported_regions_fall_back_to_the_home_region() {
        assert_eq!(
            EndpointKind::Cli.url("evil.example.com/"),
            "https://q.us-east-1.amazonaws.com/"
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
        // The legacy AWS plane exists in more regions than the Kiro planes.
        assert!(EndpointKind::Cli.serves_region("us-west-2"));
        assert!(!EndpointKind::Ide.serves_region("us-west-2"));
    }

    // Refutes: sending plain JSON; every Kiro plane speaks awsJson1_0.
    #[test]
    fn every_plane_uses_aws_json_content_type() {
        for kind in [
            EndpointKind::Ide,
            EndpointKind::Cli,
            EndpointKind::SendMessage,
        ] {
            assert_eq!(kind.content_type(), "application/x-amz-json-1.0");
        }
        assert!(!EndpointKind::Ide.sends_api_key_token_type());
        assert!(EndpointKind::Cli.sends_api_key_token_type());
    }
}
