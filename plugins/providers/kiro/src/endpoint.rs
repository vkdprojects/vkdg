//! Endpoint selection for Kiro / Amazon Q.
//!
//! Kiro's data plane is reachable through two hosts, and they are not
//! interchangeable for every credential:
//!
//! | plane           | host                                    | credential          |
//! |-----------------|-----------------------------------------|---------------------|
//! | `Ide`           | `runtime.{region}.kiro.dev`             | OAuth account only  |
//! | `CodeWhisperer` | `codewhisperer.us-east-1.amazonaws.com` | OAuth or `ksk_` key |
//! |                 | (`q.{region}.amazonaws.com` elsewhere)  |                     |
//!
//! Both route the operation in the URL path and declare `origin: AI_EDITOR`.
//! What each request carries depends on the CREDENTIAL, not on the plane:
//! an API key must not send `profileArn` (AWS answers 403) and must send
//! `tokentype: API_KEY`; an OAuth account sends `profileArn` on both planes.
//!
//! Measured against the live service:
//!
//! 1. **`runtime.*` refuses API keys** — "profileArn is required for this
//!    request" — so a `ksk_` connection only works on `CodeWhisperer`.
//! 2. **The legacy Amazon Q protocol truncates the catalogue.** Posting to the
//!    service root with `x-amz-target` and `origin: CLI` serves only
//!    `claude-sonnet-4`, `claude-sonnet-4.5` and `claude-haiku-4.5`, answering
//!    `INVALID_MODEL_ID` for everything else — `auto` included. Routing by path
//!    with `origin: AI_EDITOR` serves all 20 ids that
//!    `GET /ListAvailableModels?origin=AI_EDITOR` reports for the same key.
//! 3. **The two planes hold separate rate-limit buckets.** Driving
//!    `runtime.kiro.dev` to 75% HTTP 429 (271 of 360 at 120 concurrent) left
//!    `codewhisperer` answering 80/80 in the same window. Under a concurrency
//!    ramp, `runtime.*` started throttling at 10 concurrent and refused ~50-66%
//!    at 100, while `codewhisperer` served 2055 requests with zero errors up to
//!    200 concurrent. Two connections on one account, one per plane, therefore
//!    add capacity instead of sharing it — which is what `endpoint:` in the
//!    connection config expresses.

use crate::auth::AUTH_API_KEY;
use crate::region::{is_valid_region, DEFAULT_REGION};

/// Regions where the Kiro planes (`runtime`, `management`, `telemetry`) exist.
/// Any other region has no `kiro.dev` host, so it must fall back.
pub const RUNTIME_REGIONS: [&str; 2] = ["us-east-1", "eu-central-1"];

/// Config value naming each plane, as accepted by `endpoint:` in a connection.
pub const ENDPOINT_RUNTIME: &str = "runtime";
pub const ENDPOINT_CODEWHISPERER: &str = "codewhisperer";

/// Which host serves the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointKind {
    /// Kiro IDE data plane. OAuth accounts only.
    Ide,
    /// `CodeWhisperer` plane. Serves both credential types.
    CodeWhisperer,
}

/// Whether a request authenticated this way carries `profileArn`.
///
/// An API key belongs to a tenant already and AWS answers 403 when one is sent;
/// an OAuth account needs it on both planes.
pub fn sends_profile_arn(auth_method: &str) -> bool {
    auth_method != AUTH_API_KEY
}

/// Whether the request carries the `tokentype: API_KEY` header.
pub fn sends_api_key_token_type(auth_method: &str) -> bool {
    auth_method == AUTH_API_KEY
}

impl EndpointKind {
    /// Plane an account uses when the connection does not name one.
    pub fn for_auth_method(auth_method: &str) -> Self {
        if auth_method == AUTH_API_KEY {
            Self::CodeWhisperer
        } else {
            Self::Ide
        }
    }

    /// Plane named by the connection config, falling back to the credential's
    /// default. `runtime` is downgraded for an API key because that host refuses
    /// one outright — a config typo must not take the connection offline.
    pub fn from_config(endpoint: Option<&str>, auth_method: &str) -> Self {
        match endpoint {
            Some(ENDPOINT_CODEWHISPERER) => Self::CodeWhisperer,
            Some(ENDPOINT_RUNTIME) if auth_method != AUTH_API_KEY => Self::Ide,
            Some(ENDPOINT_RUNTIME) => Self::CodeWhisperer,
            _ => Self::for_auth_method(auth_method),
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
            Self::CodeWhisperer if region == DEFAULT_REGION => {
                format!("https://codewhisperer.{region}.amazonaws.com/generateAssistantResponse")
            }
            Self::CodeWhisperer => {
                format!("https://q.{region}.amazonaws.com/generateAssistantResponse")
            }
        }
    }

    /// `content-type` for the request body.
    ///
    /// Every plane speaks `awsJson1_0`, including `runtime.*`: third-party gateways
    /// running on the IDE plane send this header verbatim.
    pub fn content_type(self) -> &'static str {
        "application/x-amz-json-1.0"
    }

    /// True when `region` can serve this endpoint.
    pub fn serves_region(self, region: &str) -> bool {
        match self {
            Self::Ide => RUNTIME_REGIONS.contains(&region),
            // The AWS hosts exist in more regions than the Kiro planes.
            Self::CodeWhisperer => is_valid_region(region),
        }
    }

    /// `origin` the request declares, from the service's `Origin` enum.
    ///
    /// Both planes identify as an editor. `CLI` selects the truncated Amazon Q
    /// catalogue and must not be used (see the module docs).
    pub fn origin(self) -> &'static str {
        "AI_EDITOR"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{AUTH_BUILDER_ID, AUTH_IDC, AUTH_SOCIAL};

    // Refutes: tying profileArn to the plane. An OAuth account keeps sending it
    // on CodeWhisperer (measured: 900/900 OK that way), and an API key must
    // never send it on any plane — AWS answers 403.
    #[test]
    fn profile_arn_follows_the_credential_not_the_plane() {
        for oauth in [AUTH_IDC, AUTH_BUILDER_ID, AUTH_SOCIAL] {
            assert!(sends_profile_arn(oauth));
            assert!(!sends_api_key_token_type(oauth));
        }
        assert!(!sends_profile_arn(AUTH_API_KEY));
        assert!(sends_api_key_token_type(AUTH_API_KEY));
    }

    // Refutes: pointing OAuth accounts at the AWS hosts by default (documented as
    // throttling-prone for the IDE protocol) or API keys at runtime.*, which
    // answers "profileArn is required for this request."
    #[test]
    fn each_auth_method_gets_its_own_default_host() {
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

    // Refutes: ignoring `endpoint:`, which collapses both connections of an
    // account onto one rate-limit bucket and throws away the second one.
    #[test]
    fn a_connection_can_pin_its_plane() {
        assert_eq!(
            EndpointKind::from_config(Some(ENDPOINT_CODEWHISPERER), AUTH_SOCIAL),
            EndpointKind::CodeWhisperer,
            "an OAuth account may be pinned to the CodeWhisperer plane"
        );
        assert_eq!(
            EndpointKind::from_config(Some(ENDPOINT_RUNTIME), AUTH_SOCIAL),
            EndpointKind::Ide
        );
        assert_eq!(
            EndpointKind::from_config(None, AUTH_SOCIAL),
            EndpointKind::Ide,
            "unset keeps the credential's default"
        );
        assert_eq!(
            EndpointKind::from_config(None, AUTH_API_KEY),
            EndpointKind::CodeWhisperer
        );
    }

    // Refutes: honouring `endpoint: runtime` for an API key, which that host
    // refuses outright — the connection would answer 400 on every request.
    #[test]
    fn an_api_key_never_reaches_the_runtime_plane() {
        assert_eq!(
            EndpointKind::from_config(Some(ENDPOINT_RUNTIME), AUTH_API_KEY),
            EndpointKind::CodeWhisperer
        );
    }

    // Refutes: routing by path and header at the same time, which no host accepts,
    // or declaring origin CLI, which selects the truncated catalogue.
    #[test]
    fn every_plane_uses_the_editor_protocol() {
        for kind in [EndpointKind::Ide, EndpointKind::CodeWhisperer] {
            assert!(kind
                .url("us-east-1")
                .ends_with("/generateAssistantResponse"));
            assert_eq!(kind.origin(), "AI_EDITOR");
            assert_eq!(kind.content_type(), "application/x-amz-json-1.0");
        }
    }

    // Refutes: templating a host for a region that has no kiro.dev DNS
    // (ap-southeast-1 is an inference region, never a client host).
    #[test]
    fn unsupported_regions_fall_back_to_the_home_region() {
        assert_eq!(
            EndpointKind::CodeWhisperer.url("evil.example.com/"),
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
        assert_eq!(
            EndpointKind::Ide.url("eu-central-1"),
            "https://runtime.eu-central-1.kiro.dev/generateAssistantResponse"
        );
        assert!(EndpointKind::CodeWhisperer.serves_region("us-west-2"));
        assert!(!EndpointKind::Ide.serves_region("us-west-2"));
    }
}
