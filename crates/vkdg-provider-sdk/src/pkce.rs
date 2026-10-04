//! PKCE OAuth helpers shared by all authorization-code providers.
use std::collections::HashMap;

use crate::{OAuthConfig, PkceAuthorization, ProviderError};

/// N random bytes, base64url without padding.
pub fn random_b64url<const N: usize>() -> String {
    use base64::Engine as _;
    use rand::RngCore;
    let mut bytes = [0u8; N];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Validates that `uri` is `http://localhost[:port]/callback` or
/// `http://127.0.0.1[:port]/callback`.
pub fn validated_loopback_redirect(uri: &str) -> Result<String, ProviderError> {
    let reject = || {
        ProviderError::Config(format!(
            "redirect_uri must be http://localhost[:port]/callback or \
             http://127.0.0.1[:port]/callback, got '{uri}'"
        ))
    };
    let url = reqwest::Url::parse(uri).map_err(|_| reject())?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1"));
    let plain = url.scheme() == "http"
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/callback"
        && url.query().is_none()
        && url.fragment().is_none();
    if loopback && plain {
        Ok(url.to_string())
    } else {
        Err(reject())
    }
}

/// Parse what the user pasted: bare code, `code#state`, or full callback URL.
///
/// Returns `(code, Option<state>)`.
pub fn parse_pkce_callback(pasted: &str) -> Result<(String, Option<String>), ProviderError> {
    let pasted = pasted.trim();
    if pasted.starts_with("http://") || pasted.starts_with("https://") {
        let url = reqwest::Url::parse(pasted).map_err(|e| ProviderError::Http(e.to_string()))?;
        let pairs: HashMap<_, _> = url.query_pairs().into_owned().collect();
        let code = pairs
            .get("code")
            .cloned()
            .unwrap_or_else(|| pasted.to_owned());
        let state = pairs.get("state").cloned();
        return Ok((code, state));
    }
    if let Some((code, state)) = pasted.split_once('#') {
        return Ok((code.to_owned(), Some(state.to_owned())));
    }
    Ok((pasted.to_owned(), None))
}

/// Build a [`PkceAuthorization`] from an [`OAuthConfig`].
///
/// Generates verifier, S256 challenge, and state; builds the authorize URL
/// with the standard PKCE params. Extra params from
/// `config.extra_auth_params` are appended after the standard set.
pub fn build_pkce_authorization(
    config: &OAuthConfig,
    redirect_uri: &str,
) -> Result<PkceAuthorization, ProviderError> {
    use base64::Engine as _;
    use sha2::{Digest, Sha256};

    let authorize_url = config
        .authorize_url
        .as_deref()
        .ok_or_else(|| ProviderError::Config("authorize_url is required for PKCE flow".into()))?;

    // Code verifier (RFC 7636 §4.1) — 32 random bytes, base64url-encoded.
    let verifier = random_b64url::<32>();
    // S256 challenge = BASE64URL(SHA256(verifier)).
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    let state = random_b64url::<32>();

    let mut url =
        reqwest::Url::parse(authorize_url).map_err(|e| ProviderError::Http(e.to_string()))?;

    url.query_pairs_mut()
        .append_pair("client_id", &config.client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", &config.scopes.join(" "))
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state);

    for (k, v) in &config.extra_auth_params {
        url.query_pairs_mut().append_pair(k, v);
    }

    let mut login_state = HashMap::new();
    login_state.insert("code_verifier".into(), verifier);
    login_state.insert("state".into(), state);
    login_state.insert("redirect_uri".into(), redirect_uri.to_owned());

    Ok(PkceAuthorization {
        authorize_url: url.to_string(),
        state: login_state,
    })
}
