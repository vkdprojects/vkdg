//! Region and endpoint resolution for Kiro / Amazon Q.
//!
//! A Kiro account has two independent regions: the OIDC region that issued the
//! token, and the runtime region hosting the Q Developer profile. They differ for
//! IAM Identity Center accounts (an IdC in `eu-north-1` still has its profile in
//! `us-east-1` or `eu-central-1`), so the runtime region is taken from the
//! profile ARN and never from the OIDC region.

/// Regions that host an Amazon Q Developer profile.
pub const PROFILE_REGIONS: [&str; 2] = ["us-east-1", "eu-central-1"];

/// CodeWhisperer home region, used when nothing better is known.
pub const DEFAULT_REGION: &str = "us-east-1";

/// True for a syntactically valid AWS region.
///
/// Regions reach us from stored account data, so they are validated before being
/// interpolated into a request URL.
pub fn is_valid_region(region: &str) -> bool {
    !region.is_empty()
        && region.len() <= 32
        && region
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !region.starts_with('-')
        && !region.ends_with('-')
}

/// Control-plane host for a region (`ListAvailableProfiles`, `ListAvailableModels`).
///
/// These operations only exist on the AWS `q.*` hosts, which Kiro's own firewall
/// doc calls legacy and slated for deprecation; the chat data plane has already
/// moved to `runtime.{region}.kiro.dev` (see [`crate::endpoint`]). GovCloud has no
/// `kiro.dev` DNS at all and uses the FIPS hosts.
pub fn control_plane_host(region: &str) -> String {
    if let Some(gov) = region.strip_prefix("us-gov-") {
        return format!("https://q-fips.us-gov-{gov}.amazonaws.com");
    }
    format!("https://q.{region}.amazonaws.com")
}

/// Region embedded in a CodeWhisperer profile ARN
/// (`arn:aws:codewhisperer:{region}:...`).
pub fn region_from_profile_arn(profile_arn: &str) -> Option<String> {
    let region = profile_arn
        .to_ascii_lowercase()
        .strip_prefix("arn:aws:codewhisperer:")?
        .split(':')
        .next()?
        .to_owned();
    is_valid_region(&region).then_some(region)
}

/// Runtime region for upstream calls.
///
/// Priority: the profile ARN's region (authoritative), then a stored region only
/// when it actually hosts a profile, then `us-east-1`. A stored OIDC region that
/// is not a profile region is deliberately ignored.
pub fn runtime_region(profile_arn: Option<&str>, stored_region: Option<&str>) -> String {
    if let Some(region) = profile_arn.and_then(region_from_profile_arn) {
        return region;
    }
    let stored = stored_region
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if PROFILE_REGIONS.contains(&stored.as_str()) {
        return stored;
    }
    DEFAULT_REGION.to_owned()
}

/// OIDC host for a region, used for client registration, device auth and token calls.
pub fn oidc_host(region: &str) -> String {
    let region = if is_valid_region(region) {
        region
    } else {
        DEFAULT_REGION
    };
    format!("https://oidc.{region}.amazonaws.com")
}

/// Regions to probe for `ListAvailableProfiles`, most likely first.
///
/// Profile regions come first (EU-first for an EMEA account), then the account's
/// own region as a forward-compatible fallback in case AWS ever co-locates the
/// profile with the IdC.
pub fn profile_discovery_regions(stored_region: Option<&str>) -> Vec<String> {
    let stored = stored_region
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let prefer_eu = ["eu-", "af-", "me-", "il-"]
        .iter()
        .any(|p| stored.starts_with(p));

    let mut regions: Vec<String> = if prefer_eu {
        vec!["eu-central-1".to_owned(), "us-east-1".to_owned()]
    } else {
        PROFILE_REGIONS.iter().map(|r| (*r).to_owned()).collect()
    };
    if is_valid_region(&stored) && !regions.contains(&stored) {
        regions.push(stored);
    }
    regions
}

#[cfg(test)]
mod tests {
    use super::*;

    // Refutes: trusting the stored OIDC region as the runtime region. An IdC in
    // eu-north-1 has no Q runtime there; the profile ARN is authoritative.
    #[test]
    fn profile_arn_region_beats_stored_oidc_region() {
        let arn = "arn:aws:codewhisperer:eu-central-1:123456789012:profile/ABCDEF";
        assert_eq!(
            runtime_region(Some(arn), Some("eu-north-1")),
            "eu-central-1"
        );
        assert_eq!(runtime_region(None, Some("eu-north-1")), "us-east-1");
        assert_eq!(runtime_region(None, Some("eu-central-1")), "eu-central-1");
        assert_eq!(runtime_region(None, None), "us-east-1");
    }

    // Refutes: sending control-plane calls to a kiro.dev host (they only exist on
    // q.*), or to commercial q.* for a GovCloud account (no kiro.dev DNS there).
    #[test]
    fn control_plane_uses_q_hosts_and_fips_for_govcloud() {
        assert_eq!(
            control_plane_host("us-east-1"),
            "https://q.us-east-1.amazonaws.com"
        );
        assert_eq!(
            control_plane_host("eu-central-1"),
            "https://q.eu-central-1.amazonaws.com"
        );
        assert_eq!(
            control_plane_host("us-gov-west-1"),
            "https://q-fips.us-gov-west-1.amazonaws.com"
        );
    }

    // Refutes: interpolating an unvalidated region into a request URL.
    #[test]
    fn invalid_regions_are_rejected_and_fall_back() {
        for bad in ["", "US-East-1", "eu_central_1", "a/b", "-x", "x-"] {
            assert!(!is_valid_region(bad), "{bad:?} must be rejected");
        }
        assert_eq!(oidc_host("evil.example.com/"), oidc_host("us-east-1"));
        assert_eq!(
            region_from_profile_arn("arn:aws:codewhisperer:US/x:1:p"),
            None
        );
        assert_eq!(region_from_profile_arn("not-an-arn"), None);
    }

    // Refutes: probing only the IdC region, which never hosts the profile.
    #[test]
    fn discovery_probes_profile_regions_first_eu_first_for_emea() {
        assert_eq!(
            profile_discovery_regions(Some("eu-north-1")),
            ["eu-central-1", "us-east-1", "eu-north-1"]
        );
        assert_eq!(
            profile_discovery_regions(Some("us-west-2")),
            ["us-east-1", "eu-central-1", "us-west-2"]
        );
        // A profile region is never duplicated.
        assert_eq!(
            profile_discovery_regions(Some("us-east-1")),
            ["us-east-1", "eu-central-1"]
        );
    }
}
