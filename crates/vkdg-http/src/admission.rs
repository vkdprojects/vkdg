use std::net::IpAddr;
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

/// Client IP allowlist and blocklist.
///
/// Entries are an address (`10.0.0.1`, `::1`) or a CIDR range (`10.16.0.0/12`,
/// `2001:db8::/32`). Matching compares masked bits, for IPv4 and IPv6. An
/// address that does not parse never matches, and a v4 address never matches a
/// v6 range.
#[derive(Debug, Clone, Default)]
pub struct IpPolicy {
    /// If non-empty, only IPs matching one of these entries are allowed.
    pub allowlist: Vec<String>,
    /// IPs matching any of these entries are always blocked (wins over allowlist).
    pub blocklist: Vec<String>,
}

impl IpPolicy {
    /// Build a policy, rejecting entries that are neither an address nor a CIDR
    /// range. A typo in a blocklist must not silently block nothing.
    pub fn from_config(allowlist: Vec<String>, blocklist: Vec<String>) -> Result<Self, String> {
        for entry in allowlist.iter().chain(&blocklist) {
            parse_entry(entry).ok_or_else(|| format!("invalid IP or CIDR entry: {entry:?}"))?;
        }
        Ok(Self {
            allowlist,
            blocklist,
        })
    }

    /// Like [`allows`](Self::allows) for a possibly unknown client address:
    /// unknown fails an allowlist (it cannot prove membership) but passes a
    /// blocklist-only policy.
    pub fn allows_opt(&self, ip: Option<&str>) -> bool {
        match ip {
            Some(ip) => self.allows(ip),
            None => self.allowlist.is_empty(),
        }
    }

    /// Returns `true` if `ip` is allowed through this policy.
    ///
    /// Blocklist first (blocklist wins), then allowlist. An empty allowlist
    /// means "allow all" (only the blocklist applies).
    pub fn allows(&self, ip: &str) -> bool {
        let Ok(addr) = ip.trim().parse::<IpAddr>() else {
            return self.allowlist.is_empty() && self.blocklist.is_empty();
        };
        if self.blocklist.iter().any(|e| entry_contains(e, addr)) {
            return false;
        }
        self.allowlist.is_empty() || self.allowlist.iter().any(|e| entry_contains(e, addr))
    }
}

/// `addr/prefix` for an entry; a bare address is a full-length prefix.
fn parse_entry(entry: &str) -> Option<(IpAddr, u32)> {
    let entry = entry.trim();
    let (base, bits) = match entry.split_once('/') {
        Some((b, n)) => (b.parse::<IpAddr>().ok()?, n.parse::<u32>().ok()?),
        None => {
            let a = entry.parse::<IpAddr>().ok()?;
            (a, if a.is_ipv4() { 32 } else { 128 })
        }
    };
    let max = if base.is_ipv4() { 32 } else { 128 };
    (bits <= max).then_some((base, bits))
}

/// True when `addr` falls in `entry`. Also used for trusted-proxy checks.
pub(crate) fn entry_contains(entry: &str, addr: IpAddr) -> bool {
    let Some((base, bits)) = parse_entry(entry) else {
        return false;
    };
    match (base, addr) {
        (IpAddr::V4(b), IpAddr::V4(a)) => {
            masked_eq(u32::from(b).into(), u32::from(a).into(), bits, 32)
        }
        (IpAddr::V6(b), IpAddr::V6(a)) => masked_eq(u128::from(b), u128::from(a), bits, 128),
        _ => false,
    }
}

fn masked_eq(base: u128, addr: u128, bits: u32, width: u32) -> bool {
    if bits == 0 {
        return true;
    }
    let shift = width - bits;
    (base >> shift) == (addr >> shift)
}

/// String form used by callers holding an address as text.
pub(crate) fn ip_matches(ip: &str, entry: &str) -> bool {
    ip.trim()
        .parse::<IpAddr>()
        .is_ok_and(|addr| entry_contains(entry, addr))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Plausible wrong impl: blocklist doesn't override allowlist.
    #[test]
    fn blocklist_wins_over_allowlist() {
        let p = IpPolicy {
            allowlist: vec!["192.168.1.0/24".into()],
            blocklist: vec!["192.168.1.100".into()],
        };
        assert!(
            !p.allows("192.168.1.100"),
            "blocked IP must be rejected even if in allowlist range"
        );
        assert!(
            p.allows("192.168.1.200"),
            "non-blocked IP in allowlist range must be allowed"
        );
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
            allowlist: vec!["192.168.0.0/16".into()],
            blocklist: vec![],
        };
        assert!(p.allows("192.168.1.1"));
        assert!(!p.allows("192.169.1.1"), "adjacent subnet must not match");
    }

    // Plausible wrong impl: exact entry "1.2.3" matches "1.2.30.4".
    #[test]
    fn exact_entry_requires_dot_boundary() {
        let p = IpPolicy {
            allowlist: vec!["1.2.3.0/24".into()],
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

    fn allow(entries: &[&str]) -> IpPolicy {
        IpPolicy {
            allowlist: entries.iter().map(|e| (*e).to_owned()).collect(),
            blocklist: vec![],
        }
    }

    // Octet-granular matching treated /12 as /8: 10.32.0.1 got in.
    #[test]
    fn cidr_masks_bits_not_whole_octets() {
        let p = allow(&["10.16.0.0/12"]);
        assert!(p.allows("10.16.0.1"));
        assert!(p.allows("10.31.255.255"));
        assert!(!p.allows("10.32.0.1"));
        assert!(!p.allows("10.15.255.255"));
        let p = allow(&["192.168.0.0/20"]);
        assert!(p.allows("192.168.15.1"));
        assert!(!p.allows("192.168.16.1"));
    }

    #[test]
    fn ipv6_entries_match_ipv6_addresses() {
        let p = allow(&["2001:db8::/32", "::1"]);
        assert!(p.allows("2001:db8:abcd::1"));
        assert!(p.allows("::1"));
        assert!(!p.allows("2001:db9::1"));
        assert!(!p.allows("10.0.0.1"), "v4 never matches a v6 range");
    }

    // String prefixes let "10.0.0.1" match "10.0.0.1.evil" and garbage through.
    #[test]
    fn unparsable_addresses_never_match() {
        let p = allow(&["10.0.0.1"]);
        assert!(p.allows("10.0.0.1"));
        assert!(!p.allows("10.0.0.10"));
        assert!(!p.allows("10.0.0.1.evil"));
        assert!(!p.allows("not-an-ip"));
    }

    // An allowlist that an unknown client IP skips is not an allowlist.
    #[test]
    fn unknown_ip_is_rejected_when_an_allowlist_exists() {
        assert!(!allow(&["10.0.0.0/8"]).allows_opt(None));
        assert!(
            IpPolicy::default().allows_opt(None),
            "no allowlist: nothing to enforce"
        );
        let block_only = IpPolicy {
            allowlist: vec![],
            blocklist: vec!["10.0.0.0/8".into()],
        };
        assert!(block_only.allows_opt(None));
    }
}
