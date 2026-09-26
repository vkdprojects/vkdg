use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use vkdg_core::VkdgError;

pub struct AdmissionGuard {
    semaphore: Arc<Semaphore>,
}

impl AdmissionGuard {
    pub fn new(limit: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(limit)),
        }
    }

    pub fn acquire(&self) -> Result<OwnedSemaphorePermit, VkdgError> {
        self.semaphore
            .clone()
            .try_acquire_owned()
            .map_err(|_| VkdgError::AdmissionRejected {
                reason: "server at capacity".to_string(),
            })
    }
}

// ── IP policy ─────────────────────────────────────────────────────────────────

/// Simple IP policy: allowlist (if non-empty) and blocklist.
///
/// Matching is prefix-based: an entry of `"192.168.1."` matches any address
/// that starts with that prefix; an entry of `"192.168.1.10"` matches only
/// that exact address.  CIDR notation (`"10.0.0.0/8"`) is accepted: the host
/// portion after `/` is stripped and the resulting prefix is used.
///
/// Phase E: replace with a proper CIDR library for correct bit-level masking.
#[derive(Debug, Clone, Default)]
pub struct IpPolicy {
    /// If non-empty, only IPs matching one of these prefixes are allowed.
    pub allowlist: Vec<String>,
    /// IPs matching any of these prefixes are always blocked (wins over allowlist).
    pub blocklist: Vec<String>,
}

impl IpPolicy {
    pub fn from_config(allowlist: Vec<String>, blocklist: Vec<String>) -> Self {
        Self { allowlist, blocklist }
    }

    /// Returns `true` if `ip` is allowed through this policy.
    ///
    /// Evaluation order: blocklist first (blocklist wins), then allowlist.
    /// An empty allowlist means "allow all" (only blocklist applies).
    pub fn allows(&self, ip: &str) -> bool {
        // Blocklist wins — checked first.
        if self.blocklist.iter().any(|entry| ip_matches(ip, entry)) {
            return false;
        }
        // Empty allowlist = allow all.
        if self.allowlist.is_empty() {
            return true;
        }
        self.allowlist.iter().any(|entry| ip_matches(ip, entry))
    }
}

/// Returns true when `ip` matches `entry`.
///
/// - CIDR (`"10.0.0.0/8"`): use the prefix length to determine how many full
///   octets to compare.  `/8` → first octet, `/16` → first two, `/24` → first
///   three, `/32` → exact match.  Bit-level masking within an octet is not
///   performed (Phase E: replace with a proper CIDR library).
/// - Bare prefix ending with `.` (`"192.168.1."`): `ip.starts_with(entry)`.
/// - Exact address (`"10.0.0.1"`): `ip == entry` OR `ip.starts_with("10.0.0.1.")`.
///   The dot-suffix check prevents `"1.2.3"` matching `"1.2.30.4"`.
fn ip_matches(ip: &str, entry: &str) -> bool {
    if let Some(slash_pos) = entry.find('/') {
        let base = &entry[..slash_pos];
        let bits: u32 = entry[slash_pos + 1..].parse().unwrap_or(32);
        let octets_covered = (bits / 8) as usize;
        if octets_covered >= 4 {
            return ip == base;
        }
        // Collect only the first `octets_covered` octets of the base address,
        // then check that ip starts with that dotted prefix followed by a dot.
        let needed: String = base
            .split('.')
            .take(octets_covered)
            .collect::<Vec<_>>()
            .join(".");
        ip == needed.as_str() || ip.starts_with(&format!("{needed}."))
    } else if entry.ends_with('.') {
        ip.starts_with(entry)
    } else {
        ip == entry || ip.starts_with(&format!("{entry}."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Plausible wrong impl: blocklist doesn't override allowlist.
    #[test]
    fn blocklist_wins_over_allowlist() {
        let p = IpPolicy {
            allowlist: vec!["192.168.1.".into()],
            blocklist: vec!["192.168.1.100".into()],
        };
        assert!(!p.allows("192.168.1.100"), "blocked IP must be rejected even if in allowlist range");
        assert!(p.allows("192.168.1.200"), "non-blocked IP in allowlist range must be allowed");
    }

    // Plausible wrong impl: empty allowlist blocks all instead of allowing all.
    #[test]
    fn empty_allowlist_allows_all() {
        let p = IpPolicy::default();
        assert!(p.allows("1.2.3.4"));
        assert!(p.allows("192.168.1.1"));
    }

    // Plausible wrong impl: blocklist not checked when allowlist is empty.
    #[test]
    fn blocklist_blocks_without_allowlist() {
        let p = IpPolicy {
            allowlist: vec![],
            blocklist: vec!["10.0.0.1".into()],
        };
        assert!(!p.allows("10.0.0.1"));
        assert!(p.allows("10.0.0.2"));
    }

    // Plausible wrong impl: prefix "192.168" matches "192.1681.2.3" (no boundary).
    #[test]
    fn prefix_must_be_segment_boundary() {
        let p = IpPolicy {
            allowlist: vec!["192.168.".into()],
            blocklist: vec![],
        };
        assert!(p.allows("192.168.1.1"));
        assert!(!p.allows("192.169.1.1"), "adjacent subnet must not match");
    }

    // Plausible wrong impl: exact entry "1.2.3" matches "1.2.30.4".
    #[test]
    fn exact_entry_requires_dot_boundary() {
        let p = IpPolicy {
            allowlist: vec!["1.2.3".into()],
            blocklist: vec![],
        };
        assert!(p.allows("1.2.3.4"), "sub-address of entry should match");
        assert!(!p.allows("1.2.30.4"), "different octet must not match");
    }

    // Plausible wrong impl: CIDR entry fails to parse and always returns false.
    #[test]
    fn cidr_notation_uses_base_address_as_prefix() {
        let p = IpPolicy {
            allowlist: vec!["10.0.0.0/8".into()],
            blocklist: vec![],
        };
        assert!(p.allows("10.0.0.1"));
        assert!(p.allows("10.255.255.254"));
        assert!(!p.allows("11.0.0.1"));
    }
}
