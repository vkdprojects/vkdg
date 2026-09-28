//! Kiro / Amazon Q authentication: interactive login and token refresh.
//!
//! Kiro reaches the same API through five different identity systems, so an
//! account records which one issued its token in `extra["auth_method"]` and every
//! refresh follows that account's own path:
//!
//! | `auth_method`  | login                                    | refresh                     |
//! |----------------|------------------------------------------|-----------------------------|
//! | `builder-id`   | AWS Builder ID device code               | AWS SSO OIDC token grant    |
//! | `idc`          | IAM Identity Center device code          | AWS SSO OIDC token grant    |
//! | `social`       | Google/GitHub via the Kiro auth service  | Kiro `/refreshToken`        |
//! | `external_idp` | imported org (Entra) refresh token       | org IdP form-encoded grant  |
//! | `api_key`      | long-lived key, nothing to refresh       | none                        |
//!
//! Only IdC accounts need a `profileArn`; discovery is best-effort and never
//! blocks a login.

use std::collections::HashMap;
use std::time::Duration;

use futures::future::BoxFuture;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use vkdg_provider_sdk::{
    DeviceAuthorization, DevicePoll, LoginField, LoginMethod, LoginParams, LoginResult, LoginState,
    OAuthConfig, OAuthFlow, OAuthProvider, ProviderError, TokenPair,
};

use crate::region::{
    control_plane_host, is_valid_region, oidc_host, profile_discovery_regions, DEFAULT_REGION,
};
use crate::KiroAdapter;

// ── Endpoints and client registration (OmniRoute KIRO_CONFIG) ─────────────────

const CLIENT_NAME: &str = "kiro-oauth-client";
const CLIENT_TYPE: &str = "public";
const BUILDER_ID_START_URL: &str = "https://view.awsapps.com/start";
const ISSUER_URL: &str = "https://identitycenter.amazonaws.com/ssoins-722374e8c3c8e6c6";
const SCOPES: [&str; 3] = [
    "codewhisperer:completions",
    "codewhisperer:analysis",
    "codewhisperer:conversations",
];
const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

const SOCIAL_DEVICE_AUTHORIZE_URL: &str =
    "https://prod.us-east-1.auth.desktop.kiro.dev/oauth/device/authorization";
const SOCIAL_DEVICE_POLL_URL: &str =
    "https://prod.us-east-1.auth.desktop.kiro.dev/oauth/device/poll";
const SOCIAL_REFRESH_URL: &str = "https://prod.us-east-1.auth.desktop.kiro.dev/refreshToken";
const SOCIAL_CLIENT_ID: &str = "kiro-cli";

/// Token lifetime assumed when the social login does not report one.
///
/// Its refresh endpoint returns `expiresIn: 3600`, so an hour matches what the
/// service actually issues.
const SOCIAL_ASSUMED_TTL_SECS: u64 = 3600;

/// Profile discovery must never hang a login.
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);

// ── Auth method identifiers, as persisted in `extra["auth_method"]` ───────────

pub const AUTH_BUILDER_ID: &str = "builder-id";
pub const AUTH_IDC: &str = "idc";
pub const AUTH_SOCIAL: &str = "social";
pub const AUTH_EXTERNAL_IDP: &str = "external_idp";
pub const AUTH_API_KEY: &str = "api_key";

// ── Small helpers ─────────────────────────────────────────────────────────────

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

fn http_err(e: reqwest::Error) -> ProviderError {
    ProviderError::Http(e.to_string())
}

fn field(id: &str, label: &str, required: bool, secret: bool, default: Option<&str>) -> LoginField {
    LoginField {
        id: id.to_owned(),
        label: label.to_owned(),
        required,
        secret,
        default: default.map(str::to_owned),
        placeholder: None,
    }
}

fn get<'a>(map: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    map.get(key).map(String::as_str).filter(|v| !v.is_empty())
}

fn required<'a>(params: &'a LoginParams, key: &str) -> Result<&'a str, ProviderError> {
    get(params, key).ok_or_else(|| ProviderError::Http(format!("missing `{key}`")))
}

/// Validated region from params/state, falling back to the home region.
fn region_or_default(map: &HashMap<String, String>, key: &str) -> String {
    match get(map, key).map(str::to_ascii_lowercase) {
        Some(r) if is_valid_region(&r) => r,
        _ => DEFAULT_REGION.to_owned(),
    }
}

async fn json_post(url: &str, body: &Value) -> Result<(bool, Value), ProviderError> {
    let (status, value) = json_post_status(url, body).await?;
    Ok(((200..300).contains(&status), value))
}

async fn json_post_status(url: &str, body: &Value) -> Result<(u16, Value), ProviderError> {
    let resp = client()
        .post(url)
        .header("accept", "application/json")
        .json(body)
        .send()
        .await
        .map_err(http_err)?;
    let status = resp.status().as_u16();
    // Error bodies are JSON too (AWS uses `__type`), so parse either way.
    let value = resp.json::<Value>().await.unwrap_or(Value::Null);
    Ok((status, value))
}

/// Error codes meaning the refresh token itself is dead, not a passing failure.
const REVOKED_CODES: &[&str] = &[
    "invalid_grant",
    "InvalidGrantException",
    "ExpiredTokenException",
];

/// Classify a failed refresh response.
///
/// A 401 or a revoked-token error code becomes [`ProviderError::CredentialRevoked`]
/// so the gateway stops retrying and asks for a new login. Anything else (5xx,
/// throttling, malformed body) stays a retryable `TokenRefresh`. Either way the
/// upstream's own message is kept, so logs read `401: Bad credentials` rather
/// than a bare code.
fn refresh_failure(what: &str, status: u16, value: &Value) -> ProviderError {
    let code = error_code(value);
    let detail = ["message", "Message", "error_description"]
        .iter()
        .find_map(|k| str_field(value, k))
        .map_or_else(|| code.clone(), |m| format!("{code}: {m}"));
    if status == 401 || REVOKED_CODES.contains(&code.as_str()) {
        ProviderError::CredentialRevoked {
            status,
            message: format!("{what}: {detail}"),
        }
    } else {
        ProviderError::TokenRefresh(format!("{what} failed ({status}): {detail}"))
    }
}

fn str_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)?
        .as_str()
        .map(str::to_owned)
        .filter(|s| !s.is_empty())
}

/// The error code in an AWS SSO OIDC or OAuth2 response body.
fn error_code(value: &Value) -> String {
    for key in ["error", "__type", "status"] {
        if let Some(code) = str_field(value, key) {
            // AWS sends `InvalidGrantException` or a fully qualified `com.amazonaws...#Name`.
            return code.rsplit('#').next().unwrap_or(&code).to_owned();
        }
    }
    "authorization_failed".to_owned()
}

/// Short, stable account label for a credential we cannot name otherwise.
fn fingerprint(secret: &str) -> String {
    let digest = Sha256::digest(secret.as_bytes());
    digest[..4].iter().map(|b| format!("{b:02x}")).collect()
}

// ── AWS SSO OIDC: client registration, device code, token ─────────────────────

/// A dynamically registered public OIDC client.
struct RegisteredClient {
    client_id: String,
    client_secret: String,
    secret_expires_at: Option<String>,
}

/// Registers a public client with AWS SSO OIDC.
///
/// `issuer_url` is omitted for IdC tenants: a fixed issuer makes their device
/// authorization fail with `invalid_request`.
async fn register_client(
    region: &str,
    with_issuer: bool,
) -> Result<RegisteredClient, ProviderError> {
    let mut body = json!({
        "clientName": CLIENT_NAME,
        "clientType": CLIENT_TYPE,
        "scopes": SCOPES,
        "grantTypes": [DEVICE_CODE_GRANT, "refresh_token"],
    });
    if with_issuer {
        body["issuerUrl"] = json!(ISSUER_URL);
    }

    let url = format!("{}/client/register", oidc_host(region));
    let (ok, value) = json_post(&url, &body).await?;
    if !ok {
        return Err(ProviderError::Http(format!(
            "kiro client registration failed: {}",
            error_code(&value)
        )));
    }
    Ok(RegisteredClient {
        client_id: str_field(&value, "clientId")
            .ok_or_else(|| ProviderError::Http("registration returned no clientId".into()))?,
        client_secret: str_field(&value, "clientSecret").unwrap_or_default(),
        secret_expires_at: value
            .get("clientSecretExpiresAt")
            .map(|v| v.to_string().trim_matches('"').to_owned()),
    })
}

/// One AWS SSO OIDC token call (device-code exchange or refresh).
async fn oidc_token(region: &str, body: &Value) -> Result<(u16, Value), ProviderError> {
    json_post_status(&format!("{}/token", oidc_host(region)), body).await
}

/// Best-effort `ListAvailableProfiles` across the regions that can host a profile.
///
/// Returns `None` when the account has no profile (every Builder ID account) or
/// the lookup fails; the caller proceeds without one.
async fn discover_profile_arn(access_token: &str, stored_region: Option<&str>) -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(DISCOVERY_TIMEOUT)
        .build()
        .ok()?;

    for region in profile_discovery_regions(stored_region) {
        let resp = client
            .post(format!("{}/", control_plane_host(&region)))
            .header("content-type", "application/x-amz-json-1.0")
            .header("accept", "application/json")
            .header(
                "x-amz-target",
                "AmazonCodeWhispererService.ListAvailableProfiles",
            )
            .bearer_auth(access_token)
            .json(&json!({ "maxResults": 10 }))
            .send()
            .await;

        let Ok(resp) = resp else { continue };
        if !resp.status().is_success() {
            continue;
        }
        let Ok(value) = resp.json::<Value>().await else {
            continue;
        };
        let profiles = value.get("profiles").and_then(Value::as_array)?.clone();
        // Prefer a profile that lives in the region we asked, else take the first.
        let matched = profiles
            .iter()
            .find(|p| {
                str_field(p, "arn")
                    .and_then(|arn| crate::region::region_from_profile_arn(&arn))
                    .is_some_and(|r| r == region)
            })
            .or_else(|| profiles.first());
        if let Some(arn) = matched.and_then(|p| str_field(p, "arn")) {
            return Some(arn);
        }
    }
    None
}

/// Builds the account record for a completed AWS SSO OIDC login.
async fn oidc_login_result(
    tokens: &Value,
    auth_method: &str,
    region: &str,
    client_id: &str,
    client_secret: &str,
    secret_expires_at: Option<&str>,
) -> Result<LoginResult, ProviderError> {
    let access_token = str_field(tokens, "accessToken")
        .ok_or_else(|| ProviderError::Http("token response had no accessToken".into()))?;

    let mut extra = HashMap::from([
        ("auth_method".to_owned(), auth_method.to_owned()),
        ("oidc_region".to_owned(), region.to_owned()),
        ("client_id".to_owned(), client_id.to_owned()),
        ("client_secret".to_owned(), client_secret.to_owned()),
    ]);
    if let Some(expires) = secret_expires_at {
        extra.insert("client_secret_expires_at".to_owned(), expires.to_owned());
    }

    // Builder ID accounts have no Q Developer profile; only probe for IdC.
    let profile_arn = if auth_method == AUTH_IDC {
        discover_profile_arn(&access_token, Some(region)).await
    } else {
        None
    };
    let label = match &profile_arn {
        Some(arn) => {
            extra.insert("profile_arn".to_owned(), arn.clone());
            arn.rsplit('/').next().unwrap_or(arn).to_owned()
        }
        None => format!("{auth_method} ({region})"),
    };

    Ok(LoginResult {
        tokens: TokenPair {
            access_token,
            refresh_token: str_field(tokens, "refreshToken"),
            expires_in_secs: tokens.get("expiresIn").and_then(Value::as_u64),
            extra,
        },
        label,
    })
}

// ── Refresh paths ─────────────────────────────────────────────────────────────

/// Preserves account data the next refresh still needs.
fn carry_over(extra: &HashMap<String, String>, keys: &[&str]) -> HashMap<String, String> {
    keys.iter()
        .filter_map(|k| extra.get(*k).map(|v| ((*k).to_owned(), v.clone())))
        .collect()
}

/// AWS SSO OIDC refresh (Builder ID and IdC).
///
/// A registered client secret can expire or be revoked server-side while the
/// refresh token is still valid, so one failure triggers a client
/// re-registration and a single retry; the new client is persisted.
async fn refresh_oidc(
    refresh_token: &str,
    extra: &HashMap<String, String>,
) -> Result<TokenPair, ProviderError> {
    let region = region_or_default(extra, "oidc_region");
    let client_id = get(extra, "client_id").unwrap_or_default();
    let client_secret = get(extra, "client_secret").unwrap_or_default();

    let body = |id: &str, secret: &str| {
        json!({
            "clientId": id,
            "clientSecret": secret,
            "refreshToken": refresh_token,
            "grantType": "refresh_token",
        })
    };

    let mut carried = carry_over(
        extra,
        &[
            "auth_method",
            "oidc_region",
            "client_id",
            "client_secret",
            "client_secret_expires_at",
            "profile_arn",
        ],
    );

    let (status, value) = oidc_token(&region, &body(client_id, client_secret)).await?;
    let value = if (200..300).contains(&status) {
        value
    } else {
        // Re-register once, then retry with the fresh client.
        let fresh = register_client(&region, true)
            .await
            .map_err(|_| refresh_failure("kiro refresh", status, &value))?;
        let (retry_status, retry) =
            oidc_token(&region, &body(&fresh.client_id, &fresh.client_secret)).await?;
        if !(200..300).contains(&retry_status) {
            return Err(refresh_failure(
                "kiro refresh after client re-registration",
                retry_status,
                &retry,
            ));
        }
        carried.insert("client_id".to_owned(), fresh.client_id);
        carried.insert("client_secret".to_owned(), fresh.client_secret);
        if let Some(expires) = fresh.secret_expires_at {
            carried.insert("client_secret_expires_at".to_owned(), expires);
        }
        retry
    };

    Ok(TokenPair {
        access_token: str_field(&value, "accessToken")
            .ok_or_else(|| ProviderError::TokenRefresh("refresh returned no accessToken".into()))?,
        // AWS may rotate the refresh token; keep the current one when it does not.
        refresh_token: str_field(&value, "refreshToken").or_else(|| Some(refresh_token.to_owned())),
        expires_in_secs: value.get("expiresIn").and_then(Value::as_u64),
        extra: carried,
    })
}

/// Kiro social refresh (Google/GitHub accounts).
async fn refresh_social(
    refresh_token: &str,
    extra: &HashMap<String, String>,
) -> Result<TokenPair, ProviderError> {
    let (status, value) = json_post_status(
        SOCIAL_REFRESH_URL,
        &json!({ "refreshToken": refresh_token }),
    )
    .await?;
    if !(200..300).contains(&status) {
        return Err(refresh_failure("kiro social refresh", status, &value));
    }
    Ok(TokenPair {
        access_token: str_field(&value, "accessToken")
            .ok_or_else(|| ProviderError::TokenRefresh("refresh returned no accessToken".into()))?,
        refresh_token: str_field(&value, "refreshToken").or_else(|| Some(refresh_token.to_owned())),
        expires_in_secs: value.get("expiresIn").and_then(Value::as_u64),
        extra: carry_over(extra, &["auth_method", "provider", "profile_arn"]),
    })
}

/// Organization IdP refresh (Entra "Your organization" logins).
///
/// A public-client, form-encoded `refresh_token` grant against the org's own
/// token endpoint: no client secret, and never an AWS endpoint.
async fn refresh_external_idp(
    refresh_token: &str,
    extra: &HashMap<String, String>,
) -> Result<TokenPair, ProviderError> {
    let token_endpoint = get(extra, "token_endpoint").ok_or_else(|| {
        ProviderError::TokenRefresh("external_idp account has no token_endpoint".into())
    })?;
    let client_id = get(extra, "client_id").ok_or_else(|| {
        ProviderError::TokenRefresh("external_idp account has no client_id".into())
    })?;

    let mut form = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
    ];
    if let Some(scope) = get(extra, "scope") {
        form.push(("scope", scope));
    }

    let resp = client()
        .post(token_endpoint)
        .header("accept", "application/json")
        .form(&form)
        .send()
        .await
        .map_err(http_err)?;
    let status = resp.status().as_u16();
    let value = resp.json::<Value>().await.unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        return Err(refresh_failure("kiro external_idp refresh", status, &value));
    }

    Ok(TokenPair {
        // Org IdPs use snake_case OAuth2 field names, not AWS camelCase.
        access_token: str_field(&value, "access_token").ok_or_else(|| {
            ProviderError::TokenRefresh("refresh returned no access_token".into())
        })?,
        refresh_token: str_field(&value, "refresh_token")
            .or_else(|| Some(refresh_token.to_owned())),
        expires_in_secs: value.get("expires_in").and_then(Value::as_u64),
        extra: carry_over(
            extra,
            &[
                "auth_method",
                "token_endpoint",
                "client_id",
                "scope",
                "profile_arn",
            ],
        ),
    })
}

// ── OAuthProvider ─────────────────────────────────────────────────────────────

impl OAuthProvider for KiroAdapter {
    fn oauth_config(&self) -> OAuthConfig {
        OAuthConfig {
            flow: OAuthFlow::DeviceCode,
            authorize_url: None,
            // Default region; an IdC connection may use a regional endpoint.
            token_url: format!("{}/token", oidc_host(DEFAULT_REGION)),
            // Clients are registered per login, so there is no static id.
            client_id: String::new(),
            scopes: SCOPES.iter().map(|s| (*s).to_owned()).collect(),
            redirect_uri: None,
            extra_auth_params: HashMap::new(),
        }
    }

    fn login_methods(&self) -> Vec<LoginMethod> {
        vec![
            LoginMethod {
                id: AUTH_BUILDER_ID.to_owned(),
                label: "AWS Builder ID (free)".to_owned(),
                flow: OAuthFlow::DeviceCode,
                fields: vec![field(
                    "region",
                    "AWS region",
                    false,
                    false,
                    Some(DEFAULT_REGION),
                )],
                hint: None,
                icon_char: None,
            },
            LoginMethod {
                id: AUTH_IDC.to_owned(),
                label: "IAM Identity Center (organization)".to_owned(),
                flow: OAuthFlow::DeviceCode,
                fields: vec![
                    field("start_url", "IdC start URL", true, false, None),
                    field("region", "IdC region", true, false, Some(DEFAULT_REGION)),
                ],
                hint: None,
                icon_char: None,
            },
            LoginMethod {
                id: AUTH_SOCIAL.to_owned(),
                label: "Google or GitHub".to_owned(),
                flow: OAuthFlow::DeviceCode,
                fields: vec![field(
                    "provider",
                    "Identity provider (Google or Github)",
                    true,
                    false,
                    Some("Google"),
                )],
                hint: None,
                icon_char: None,
            },
            LoginMethod {
                id: "import".to_owned(),
                label: "Paste an existing refresh token".to_owned(),
                flow: OAuthFlow::ImportToken,
                fields: vec![
                    field("refresh_token", "Refresh token", true, true, None),
                    field("region", "AWS region", false, false, Some(DEFAULT_REGION)),
                    field(
                        "client_id",
                        "Client id (IdC or org IdP)",
                        false,
                        false,
                        None,
                    ),
                    field("client_secret", "Client secret (IdC)", false, true, None),
                    field(
                        "token_endpoint",
                        "Token endpoint (organization IdP only)",
                        false,
                        false,
                        None,
                    ),
                    field("scope", "Scope (organization IdP only)", false, false, None),
                ],
                hint: None,
                icon_char: None,
            },
            LoginMethod {
                id: AUTH_API_KEY.to_owned(),
                label: "Long-lived API key".to_owned(),
                flow: OAuthFlow::ImportToken,
                fields: vec![
                    field("api_key", "API key", true, true, None),
                    field("region", "AWS region", false, false, Some(DEFAULT_REGION)),
                ],
                hint: None,
                icon_char: None,
            },
        ]
    }

    fn start_device_login<'a>(
        &'a self,
        method: &'a str,
        params: &'a LoginParams,
    ) -> BoxFuture<'a, Result<DeviceAuthorization, ProviderError>> {
        Box::pin(async move {
            if method == AUTH_SOCIAL {
                return start_social_device_login(params).await;
            }

            let region = region_or_default(params, "region");
            let is_idc = method == AUTH_IDC;
            let start_url = if is_idc {
                required(params, "start_url")?.to_owned()
            } else {
                BUILDER_ID_START_URL.to_owned()
            };

            // IdC tenants have their own issuer; sending a fixed one breaks them.
            let client = register_client(&region, !is_idc).await?;
            let (ok, value) = json_post(
                &format!("{}/device_authorization", oidc_host(&region)),
                &json!({
                    "clientId": client.client_id,
                    "clientSecret": client.client_secret,
                    "startUrl": start_url,
                }),
            )
            .await?;
            if !ok {
                return Err(ProviderError::Http(format!(
                    "kiro device authorization failed: {}",
                    error_code(&value)
                )));
            }

            let mut state = LoginState::from([
                ("region".to_owned(), region),
                ("client_id".to_owned(), client.client_id),
                ("client_secret".to_owned(), client.client_secret),
                (
                    "device_code".to_owned(),
                    str_field(&value, "deviceCode").ok_or_else(|| {
                        ProviderError::Http("device authorization returned no deviceCode".into())
                    })?,
                ),
                (
                    "auth_method".to_owned(),
                    if is_idc { AUTH_IDC } else { AUTH_BUILDER_ID }.to_owned(),
                ),
            ]);
            if let Some(expires) = client.secret_expires_at {
                state.insert("client_secret_expires_at".to_owned(), expires);
            }

            Ok(DeviceAuthorization {
                verification_uri: str_field(&value, "verificationUri").ok_or_else(|| {
                    ProviderError::Http("device authorization returned no verificationUri".into())
                })?,
                verification_uri_complete: str_field(&value, "verificationUriComplete"),
                user_code: str_field(&value, "userCode").unwrap_or_default(),
                interval_secs: value.get("interval").and_then(Value::as_u64).unwrap_or(5),
                expires_in_secs: value
                    .get("expiresIn")
                    .and_then(Value::as_u64)
                    .unwrap_or(600),
                state,
            })
        })
    }

    fn poll_device_login<'a>(
        &'a self,
        method: &'a str,
        state: &'a LoginState,
    ) -> BoxFuture<'a, Result<DevicePoll, ProviderError>> {
        Box::pin(async move {
            if method == AUTH_SOCIAL {
                return poll_social_device_login(state).await;
            }

            let region = region_or_default(state, "region");
            let (status, value) = oidc_token(
                &region,
                &json!({
                    "clientId": get(state, "client_id").unwrap_or_default(),
                    "clientSecret": get(state, "client_secret").unwrap_or_default(),
                    "deviceCode": get(state, "device_code").unwrap_or_default(),
                    "grantType": DEVICE_CODE_GRANT,
                }),
            )
            .await?;

            if (200..300).contains(&status) && value.get("accessToken").is_some() {
                let auth_method = get(state, "auth_method").unwrap_or(AUTH_BUILDER_ID);
                return oidc_login_result(
                    &value,
                    auth_method,
                    &region,
                    get(state, "client_id").unwrap_or_default(),
                    get(state, "client_secret").unwrap_or_default(),
                    get(state, "client_secret_expires_at"),
                )
                .await
                .map(DevicePoll::Done);
            }

            Ok(classify_poll(&error_code(&value)))
        })
    }

    fn import_token<'a>(
        &'a self,
        method: &'a str,
        params: &'a LoginParams,
    ) -> BoxFuture<'a, Result<LoginResult, ProviderError>> {
        Box::pin(async move {
            let region = region_or_default(params, "region");

            // An API key is used verbatim: there is nothing to exchange or refresh.
            if method == AUTH_API_KEY {
                let key = required(params, "api_key")?;
                return Ok(LoginResult {
                    tokens: TokenPair {
                        access_token: key.to_owned(),
                        refresh_token: None,
                        expires_in_secs: None,
                        extra: HashMap::from([
                            ("auth_method".to_owned(), AUTH_API_KEY.to_owned()),
                            ("oidc_region".to_owned(), region.clone()),
                        ]),
                    },
                    label: format!("API Key ({region}, {})", fingerprint(key)),
                });
            }

            let refresh_token = required(params, "refresh_token")?;

            // An org IdP token endpoint marks an external_idp account, which
            // refreshes against that endpoint rather than AWS.
            if let Some(token_endpoint) = get(params, "token_endpoint") {
                let mut extra = HashMap::from([
                    ("auth_method".to_owned(), AUTH_EXTERNAL_IDP.to_owned()),
                    ("token_endpoint".to_owned(), token_endpoint.to_owned()),
                    (
                        "client_id".to_owned(),
                        required(params, "client_id")?.to_owned(),
                    ),
                ]);
                if let Some(scope) = get(params, "scope") {
                    extra.insert("scope".to_owned(), scope.to_owned());
                }
                // Exchange immediately: an invalid token must fail at login, not first use.
                let tokens = refresh_external_idp(refresh_token, &extra).await?;
                return Ok(LoginResult {
                    label: format!("organization ({})", fingerprint(refresh_token)),
                    tokens,
                });
            }

            // With a client id/secret the token is AWS SSO OIDC issued; without
            // them it can only be a Kiro social token.
            let (auth_method, extra) =
                match (get(params, "client_id"), get(params, "client_secret")) {
                    (Some(client_id), Some(client_secret)) => (
                        AUTH_IDC,
                        HashMap::from([
                            ("auth_method".to_owned(), AUTH_IDC.to_owned()),
                            ("oidc_region".to_owned(), region.clone()),
                            ("client_id".to_owned(), client_id.to_owned()),
                            ("client_secret".to_owned(), client_secret.to_owned()),
                        ]),
                    ),
                    _ => (
                        AUTH_SOCIAL,
                        HashMap::from([("auth_method".to_owned(), AUTH_SOCIAL.to_owned())]),
                    ),
                };

            let mut tokens = if auth_method == AUTH_IDC {
                refresh_oidc(refresh_token, &extra).await?
            } else {
                refresh_social(refresh_token, &extra).await?
            };

            // IdC accounts need a profileArn on every call; look it up once here.
            if auth_method == AUTH_IDC {
                if let Some(arn) = discover_profile_arn(&tokens.access_token, Some(&region)).await {
                    tokens.extra.insert("profile_arn".to_owned(), arn);
                }
            }
            let label = match tokens.extra.get("profile_arn") {
                Some(arn) => arn.rsplit('/').next().unwrap_or(arn).to_owned(),
                None => format!("{auth_method} ({})", fingerprint(refresh_token)),
            };
            Ok(LoginResult { tokens, label })
        })
    }

    fn refresh_token<'a>(
        &'a self,
        refresh_token: &'a str,
        extra: &'a HashMap<String, String>,
    ) -> BoxFuture<'a, Result<TokenPair, ProviderError>> {
        Box::pin(async move {
            match get(extra, "auth_method").unwrap_or(AUTH_BUILDER_ID) {
                AUTH_API_KEY => Err(ProviderError::TokenRefresh(
                    "kiro API keys do not expire and cannot be refreshed".into(),
                )),
                AUTH_EXTERNAL_IDP => refresh_external_idp(refresh_token, extra).await,
                AUTH_SOCIAL => refresh_social(refresh_token, extra).await,
                // Builder ID and IdC both refresh against AWS SSO OIDC.
                _ => refresh_oidc(refresh_token, extra).await,
            }
        })
    }
}

// ── Social device flow (Google / GitHub via the Kiro auth service) ────────────

/// Normalises a login provider to the enum the service accepts.
///
/// The endpoint validates against exactly `[Github, Cognito, Google]` — `GitHub`
/// with a capital H is rejected — so a user-supplied value is mapped rather than
/// forwarded.
fn social_login_provider(input: &str) -> &'static str {
    match input.trim().to_ascii_lowercase().as_str() {
        "github" => "Github",
        "cognito" | "builder-id" | "builderid" => "Cognito",
        _ => "Google",
    }
}

async fn start_social_device_login(
    params: &LoginParams,
) -> Result<DeviceAuthorization, ProviderError> {
    let provider = social_login_provider(get(params, "provider").unwrap_or("Google"));
    let (ok, value) = json_post(
        SOCIAL_DEVICE_AUTHORIZE_URL,
        // The field is `loginProvider`; `provider` is rejected as a null member.
        &json!({ "clientId": SOCIAL_CLIENT_ID, "loginProvider": provider }),
    )
    .await?;
    if !ok {
        return Err(ProviderError::Http(format!(
            "kiro social device authorization failed: {}",
            // This service reports validation failures in `message`.
            str_field(&value, "message")
                .map(|m| m.to_owned())
                .unwrap_or_else(|| error_code(&value))
        )));
    }

    let device_code = str_field(&value, "deviceCode")
        .ok_or_else(|| ProviderError::Http("social authorization returned no deviceCode".into()))?;
    let verification_uri = str_field(&value, "verificationUri").ok_or_else(|| {
        ProviderError::Http("social authorization returned no verificationUri".into())
    })?;

    // This endpoint reports durations in milliseconds, unlike the AWS OIDC one.
    let secs = |key: &str, default: u64| {
        value
            .get(key)
            .and_then(Value::as_u64)
            .map_or(default, |ms| (ms / 1000).max(1))
    };

    Ok(DeviceAuthorization {
        verification_uri,
        // Carries the user code and provider, so the user only has to approve.
        verification_uri_complete: str_field(&value, "verificationUriComplete"),
        user_code: str_field(&value, "userCode").unwrap_or_default(),
        interval_secs: secs("intervalInMilliseconds", 5),
        expires_in_secs: secs("expiresInMilliseconds", 300),
        state: LoginState::from([
            ("device_code".to_owned(), device_code),
            ("provider".to_owned(), provider.to_owned()),
        ]),
    })
}

async fn poll_social_device_login(state: &LoginState) -> Result<DevicePoll, ProviderError> {
    let (ok, value) = json_post(
        SOCIAL_DEVICE_POLL_URL,
        &json!({
            "clientId": SOCIAL_CLIENT_ID,
            "deviceCode": get(state, "device_code").unwrap_or_default(),
        }),
    )
    .await?;

    // Kiro reports progress in `status`, and only real failures in `error`.
    let progress = str_field(&value, "error").or_else(|| str_field(&value, "status"));
    if let Some(code) = progress.as_deref() {
        if code == "authorization_pending" {
            return Ok(DevicePoll::Pending);
        }
        if code == "slow_down" {
            return Ok(DevicePoll::SlowDown);
        }
    }
    if !ok || value.get("error").is_some() {
        return Ok(DevicePoll::Failed(error_code(&value)));
    }

    let Some(access_token) = str_field(&value, "accessToken") else {
        return Ok(DevicePoll::Failed("invalid_token_response".to_owned()));
    };
    let provider = get(state, "provider").unwrap_or("Google").to_owned();

    let mut extra = HashMap::from([
        ("auth_method".to_owned(), AUTH_SOCIAL.to_owned()),
        ("provider".to_owned(), provider.clone()),
    ]);
    // The service hands back the profile ARN here. Social accounts still need it
    // on every request — upstream answers 400 "profileArn is required" without one
    // — and `ListAvailableProfiles` returns nothing for them, so this response is
    // the only place it can be obtained.
    if let Some(arn) = str_field(&value, "profileArn") {
        extra.insert("profile_arn".to_owned(), arn);
    }

    Ok(DevicePoll::Done(LoginResult {
        tokens: TokenPair {
            access_token,
            refresh_token: str_field(&value, "refreshToken"),
            // The social login omits `expiresIn`, though its refresh endpoint
            // reports 3600. Assume the same hour rather than leaving the expiry
            // unknown, so the gateway refreshes ahead instead of on every request.
            expires_in_secs: value
                .get("expiresIn")
                .and_then(Value::as_u64)
                .or(Some(SOCIAL_ASSUMED_TTL_SECS)),
            extra,
        },
        label: provider,
    }))
}

/// Maps a device-code error code to a poll outcome (RFC 8628 §3.5).
fn classify_poll(code: &str) -> DevicePoll {
    match code {
        "AuthorizationPendingException" | "authorization_pending" => DevicePoll::Pending,
        "SlowDownException" | "slow_down" => DevicePoll::SlowDown,
        other => DevicePoll::Failed(other.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Refutes: treating "pending" as a failure (aborting a login the user has not
    // finished) or a real error as pending (polling until the code expires).
    #[test]
    fn poll_codes_map_to_outcomes() {
        assert!(matches!(
            classify_poll("AuthorizationPendingException"),
            DevicePoll::Pending
        ));
        assert!(matches!(
            classify_poll("authorization_pending"),
            DevicePoll::Pending
        ));
        assert!(matches!(
            classify_poll("SlowDownException"),
            DevicePoll::SlowDown
        ));
        assert!(matches!(classify_poll("slow_down"), DevicePoll::SlowDown));
        match classify_poll("ExpiredTokenException") {
            DevicePoll::Failed(msg) => assert_eq!(msg, "ExpiredTokenException"),
            other => panic!("expiry must be terminal, got {other:?}"),
        }
    }

    // Refutes: reporting the raw AWS wire type, which is fully qualified.
    #[test]
    fn aws_error_types_are_unqualified() {
        let body = json!({ "__type": "com.amazonaws.ssooidc#InvalidGrantException" });
        assert_eq!(error_code(&body), "InvalidGrantException");
        assert_eq!(
            error_code(&json!({ "error": "invalid_grant" })),
            "invalid_grant"
        );
        assert_eq!(
            error_code(&json!({ "status": "authorization_pending" })),
            "authorization_pending"
        );
        assert_eq!(error_code(&Value::Null), "authorization_failed");
    }

    // Found live: GitHub social refresh answered 401 "Bad credentials" and the
    // gateway logged only `authorization_failed`, then retried every request.
    #[test]
    fn refresh_401_is_revoked_and_keeps_upstream_message() {
        let body = json!({ "message": "Bad credentials" });
        match refresh_failure("kiro social refresh", 401, &body) {
            ProviderError::CredentialRevoked { status, message } => {
                assert_eq!(status, 401);
                assert!(message.contains("Bad credentials"), "{message}");
            }
            other => panic!("expected CredentialRevoked, got {other:?}"),
        }
    }

    // AWS OIDC answers 400 with a code, not 401, for a dead refresh token.
    #[test]
    fn revoked_grant_codes_are_revoked_regardless_of_status() {
        for body in [
            json!({ "error": "invalid_grant" }),
            json!({ "__type": "com.amazonaws.ssooidc#InvalidGrantException" }),
            json!({ "__type": "ExpiredTokenException", "message": "expired" }),
        ] {
            assert!(
                matches!(
                    refresh_failure("kiro refresh", 400, &body),
                    ProviderError::CredentialRevoked { status: 400, .. }
                ),
                "{body}"
            );
        }
    }

    // Plausible wrong impl: every failure parks the account, so one AWS outage
    // forces every user to log in again.
    #[test]
    fn server_errors_and_throttling_stay_retryable() {
        for (status, body) in [
            (500, json!({ "message": "internal" })),
            (429, json!({ "__type": "ThrottlingException" })),
            (503, Value::Null),
        ] {
            match refresh_failure("kiro refresh", status, &body) {
                ProviderError::TokenRefresh(msg) => {
                    assert!(msg.contains(&status.to_string()), "{msg}")
                }
                other => panic!("{status}: expected TokenRefresh, got {other:?}"),
            }
        }
    }

    // Refutes: dropping the client registration (or profile) on refresh, which
    // makes every later refresh fail.
    #[test]
    fn carry_over_keeps_only_known_keys() {
        let extra = HashMap::from([
            ("auth_method".to_owned(), "idc".to_owned()),
            ("client_id".to_owned(), "cid".to_owned()),
            (
                "profile_arn".to_owned(),
                "arn:aws:codewhisperer:us-east-1:1:profile/P".to_owned(),
            ),
            ("access_token".to_owned(), "leak".to_owned()),
        ]);
        let carried = carry_over(&extra, &["auth_method", "client_id", "profile_arn"]);
        assert_eq!(carried.len(), 3);
        assert!(!carried.contains_key("access_token"));
        assert_eq!(carried["profile_arn"], extra["profile_arn"]);
    }

    // Refutes: advertising a method whose required fields the CLI cannot prompt for.
    #[test]
    fn every_login_method_declares_its_inputs() {
        let methods = KiroAdapter.login_methods();
        let ids: Vec<&str> = methods.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                AUTH_BUILDER_ID,
                AUTH_IDC,
                AUTH_SOCIAL,
                "import",
                AUTH_API_KEY
            ]
        );
        let idc = &methods[1];
        assert!(idc.fields.iter().any(|f| f.id == "start_url" && f.required));
        let import = &methods[3];
        let token = import
            .fields
            .iter()
            .find(|f| f.id == "refresh_token")
            .expect("import must accept a refresh_token");
        assert!(
            token.required && token.secret,
            "pasted tokens must be masked"
        );
        let api_key = &methods[4];
        assert!(api_key.fields.iter().any(|f| f.id == "api_key" && f.secret));
    }

    // Refutes: labelling every key-based account identically, so multiple
    // accounts become indistinguishable in listings.
    #[test]
    fn fingerprints_are_stable_and_distinct() {
        assert_eq!(fingerprint("key-a"), fingerprint("key-a"));
        assert_ne!(fingerprint("key-a"), fingerprint("key-b"));
        assert_eq!(fingerprint("key-a").len(), 8);
    }
}
