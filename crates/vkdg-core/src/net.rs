//! Client IP rules: typed CIDR ranges and an allow/block policy.
//!
//! Standard library only, so config validation, the HTTP front door, and
//! per-key allowlists share one parser and one matcher.

use std::fmt;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::{Arc, PoisonError, RwLock};

/// An address range: `10.16.0.0/12`, `2001:db8::/32`, or a single address
/// (`10.0.0.1`, `::1`), which is a full-length prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpNet {
    addr: IpAddr,
    prefix: u8,
}

impl IpNet {
    /// True when `ip` is inside this range. IPv4 never matches an IPv6 range
    /// and vice versa.
    pub fn contains(&self, ip: IpAddr) -> bool {
        let prefix = u32::from(self.prefix);
        match (self.addr, ip) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                same_prefix(u32::from(net).into(), u32::from(ip).into(), prefix, 32)
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                same_prefix(u128::from(net), u128::from(ip), prefix, 128)
            }
            _ => false,
        }
    }
}

fn same_prefix(net: u128, ip: u128, prefix: u32, width: u32) -> bool {
    prefix == 0 || (net >> (width - prefix)) == (ip >> (width - prefix))
}

impl FromStr for IpNet {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let (addr, prefix) = match s.split_once('/') {
            Some((addr, prefix)) => (addr, Some(prefix)),
            None => (s, None),
        };
        let addr: IpAddr = addr
            .parse()
            .map_err(|_| format!("{s:?} is not an IP address or CIDR range"))?;
        let max: u8 = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            None => max,
            Some(p) => p
                .parse::<u8>()
                .ok()
                .filter(|p| *p <= max)
                .ok_or_else(|| format!("{s:?}: prefix length must be 0-{max}"))?,
        };
        Ok(Self { addr, prefix })
    }
}

impl fmt::Display for IpNet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.addr, self.prefix)
    }
}

/// Parse a config list, naming the offending entry (`field[i]`) on error.
pub fn parse_ip_list(field: &str, entries: &[String]) -> Result<Vec<IpNet>, String> {
    entries
        .iter()
        .enumerate()
        .map(|(i, e)| e.parse().map_err(|msg| format!("{field}[{i}]: {msg}")))
        .collect()
}

/// A parsed allowlist and blocklist.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IpRules {
    /// If non-empty, only addresses in one of these ranges are allowed.
    pub allow: Vec<IpNet>,
    /// Addresses in any of these ranges are refused. Wins over `allow`.
    pub block: Vec<IpNet>,
}

impl IpRules {
    /// Whether a client may pass. An unknown address fails a non-empty
    /// allowlist (it cannot prove membership) but passes a blocklist-only rule.
    pub fn allows(&self, ip: Option<IpAddr>) -> bool {
        let Some(ip) = ip else {
            return self.allow.is_empty();
        };
        if self.block.iter().any(|n| n.contains(ip)) {
            return false;
        }
        self.allow.is_empty() || self.allow.iter().any(|n| n.contains(ip))
    }
}

/// The IP rules in force, swapped whole on config reload.
#[derive(Debug, Default)]
pub struct IpPolicy {
    rules: RwLock<Arc<IpRules>>,
}

impl IpPolicy {
    pub fn new(rules: IpRules) -> Self {
        Self {
            rules: RwLock::new(Arc::new(rules)),
        }
    }

    pub fn replace(&self, rules: IpRules) {
        *self.rules.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(rules);
    }

    pub fn allows(&self, ip: Option<IpAddr>) -> bool {
        self.rules
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .allows(ip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nets(entries: &[&str]) -> Vec<IpNet> {
        entries.iter().map(|e| e.parse().unwrap()).collect()
    }
    fn ip(s: &str) -> Option<IpAddr> {
        Some(s.parse().unwrap())
    }
    fn allow(entries: &[&str]) -> IpRules {
        IpRules {
            allow: nets(entries),
            block: vec![],
        }
    }

    // Octet-granular matching treated /12 as /8 and let 10.32.0.1 in.
    #[test]
    fn cidr_masks_bits_not_whole_octets() {
        let r = allow(&["10.16.0.0/12", "192.168.0.0/20"]);
        assert!(r.allows(ip("10.16.0.1")));
        assert!(r.allows(ip("10.31.255.255")));
        assert!(!r.allows(ip("10.32.0.1")));
        assert!(!r.allows(ip("10.15.255.255")));
        assert!(r.allows(ip("192.168.15.1")));
        assert!(!r.allows(ip("192.168.16.1")));
    }

    #[test]
    fn ipv6_ranges_never_match_ipv4() {
        let r = allow(&["2001:db8::/32", "::1"]);
        assert!(r.allows(ip("2001:db8:abcd::1")));
        assert!(r.allows(ip("::1")));
        assert!(!r.allows(ip("2001:db9::1")));
        assert!(!r.allows(ip("10.0.0.1")));
    }

    #[test]
    fn blocklist_wins_and_empty_allowlist_allows_all() {
        let r = IpRules {
            allow: nets(&["192.168.1.0/24"]),
            block: nets(&["192.168.1.100"]),
        };
        assert!(!r.allows(ip("192.168.1.100")));
        assert!(r.allows(ip("192.168.1.200")));
        let block_only = IpRules {
            allow: vec![],
            block: nets(&["10.0.0.1"]),
        };
        assert!(!block_only.allows(ip("10.0.0.1")));
        assert!(block_only.allows(ip("10.0.0.2")));
    }

    // An allowlist that an unknown client IP skips is not an allowlist.
    #[test]
    fn unknown_ip_fails_an_allowlist_but_not_a_blocklist() {
        assert!(!allow(&["10.0.0.0/8"]).allows(None));
        assert!(IpRules::default().allows(None));
        let block_only = IpRules {
            allow: vec![],
            block: nets(&["10.0.0.0/8"]),
        };
        assert!(block_only.allows(None));
    }

    // The old dotted-prefix form ("192.168.") silently matched nothing; it must
    // be a config error that names the entry.
    #[test]
    fn invalid_entries_are_rejected_with_their_position() {
        for bad in ["192.168.", "10.0.0.0/33", "::/129", "not-an-ip", ""] {
            let list = vec!["10.0.0.0/8".to_owned(), bad.to_owned()];
            let err = parse_ip_list("limits.ip_allowlist", &list).unwrap_err();
            assert!(err.starts_with("limits.ip_allowlist[1]"), "{bad:?}: {err}");
        }
    }

    #[test]
    fn policy_swaps_rules_in_place() {
        let policy = IpPolicy::new(allow(&["10.0.0.0/8"]));
        assert!(!policy.allows(ip("192.168.0.1")));
        policy.replace(allow(&["192.168.0.0/16"]));
        assert!(policy.allows(ip("192.168.0.1")));
        assert!(!policy.allows(ip("10.0.0.1")));
    }
}
