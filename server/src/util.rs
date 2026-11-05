use std::net::{IpAddr, SocketAddr};

use chrono::{DateTime, NaiveDateTime, SubsecRound, Utc};

/// Current UTC time truncated to microseconds, so a value survives a
/// database round trip and RFC 3339 formatting without changing.
pub fn now() -> NaiveDateTime {
    Utc::now().naive_utc().trunc_subsecs(6)
}

/// `2026-11-04T08:30:00.123456Z` — the protocol's canonical timestamp form.
pub fn fmt_date(date: &NaiveDateTime) -> String {
    date.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

pub fn fmt_opt_date(date: &Option<NaiveDateTime>) -> Option<String> {
    date.as_ref().map(fmt_date)
}

/// Accepts any RFC 3339 timestamp and normalises it to naive UTC.
pub fn parse_date(raw: &str) -> Option<NaiveDateTime> {
    DateTime::parse_from_rfc3339(raw.trim()).ok().map(|d| d.naive_utc().trunc_subsecs(6))
}

pub fn new_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

pub fn is_valid_email(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && email.len() <= 254
        && !email.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Validates the shape of a protocol EncString (`1.<b64 iv>.<b64 ciphertext>`).
/// The server cannot decrypt it, but rejecting garbage early keeps clients honest.
pub fn is_enc_string(value: &str, max_len: usize) -> bool {
    use data_encoding::BASE64;
    if value.len() > max_len {
        return false;
    }
    let mut parts = value.split('.');
    let (Some("1"), Some(iv), Some(ct), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let iv_ok = BASE64.decode(iv.as_bytes()).map(|b| b.len() == 12).unwrap_or(false);
    // AES-GCM output always carries a 16-byte tag; only check the encoding cheaply for large values.
    let ct_ok = ct.len() >= 24
        && ct.len() % 4 == 0
        && ct.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'=');
    iv_ok && ct_ok
}

/// An IP network such as `10.0.0.0/8` or `fd00::/8`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpNet {
    addr: IpAddr,
    prefix: u8,
}

impl IpNet {
    /// Parses `addr` or `addr/prefix`. IPv4-mapped IPv6 addresses are treated as IPv4.
    pub fn parse(raw: &str) -> Option<Self> {
        let (addr, prefix) = match raw.trim().split_once('/') {
            Some((addr, prefix)) => (addr.trim().parse::<IpAddr>().ok()?, Some(prefix.trim().parse::<u8>().ok()?)),
            None => (raw.trim().parse::<IpAddr>().ok()?, None),
        };
        let addr = addr.to_canonical();
        let max = if addr.is_ipv4() {
            32
        } else {
            128
        };
        let prefix = prefix.unwrap_or(max);
        (prefix <= max).then_some(Self {
            addr,
            prefix,
        })
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.addr, ip.to_canonical()) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                masked(u32::from(net).into(), 32, self.prefix) == masked(u32::from(ip).into(), 32, self.prefix)
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                masked(u128::from(net), 128, self.prefix) == masked(u128::from(ip), 128, self.prefix)
            }
            _ => false,
        }
    }
}

fn masked(bits: u128, width: u8, prefix: u8) -> u128 {
    if prefix == 0 {
        0
    } else {
        bits >> (width - prefix)
    }
}

/// One address from a client-IP header: `198.51.100.7`, `198.51.100.7:5000`, `2001:db8::1`
/// or `[2001:db8::1]:443`.
pub fn parse_forwarded_ip(raw: &str) -> Option<IpAddr> {
    let raw = raw.trim();
    raw.parse::<IpAddr>()
        .ok()
        .or_else(|| raw.parse::<SocketAddr>().ok().map(|a| a.ip()))
        .or_else(|| raw.strip_prefix('[')?.strip_suffix(']')?.parse().ok())
        .map(|ip| ip.to_canonical())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ip_networks() {
        let lan = IpNet::parse("192.168.0.0/16").unwrap();
        assert!(lan.contains("192.168.4.7".parse().unwrap()));
        assert!(lan.contains("::ffff:192.168.4.7".parse().unwrap()));
        assert!(!lan.contains("192.169.0.1".parse().unwrap()));
        assert!(!lan.contains("fd00::1".parse().unwrap()));
        let ula = IpNet::parse("fc00::/7").unwrap();
        assert!(ula.contains("fd12:3456::1".parse().unwrap()));
        assert!(!ula.contains("2001:db8::1".parse().unwrap()));
        let single = IpNet::parse("203.0.113.9").unwrap();
        assert!(single.contains("203.0.113.9".parse().unwrap()));
        assert!(!single.contains("203.0.113.10".parse().unwrap()));
        assert!(IpNet::parse("0.0.0.0/0").unwrap().contains("8.8.8.8".parse().unwrap()));
        assert_eq!(IpNet::parse("10.0.0.0/33"), None);
        assert_eq!(IpNet::parse("not-an-ip"), None);
    }

    #[test]
    fn forwarded_addresses() {
        let v4: IpAddr = "198.51.100.7".parse().unwrap();
        let v6: IpAddr = "2001:db8::1".parse().unwrap();
        assert_eq!(parse_forwarded_ip(" 198.51.100.7 "), Some(v4));
        assert_eq!(parse_forwarded_ip("198.51.100.7:5000"), Some(v4));
        assert_eq!(parse_forwarded_ip("::ffff:198.51.100.7"), Some(v4));
        assert_eq!(parse_forwarded_ip("2001:db8::1"), Some(v6));
        assert_eq!(parse_forwarded_ip("[2001:db8::1]:443"), Some(v6));
        assert_eq!(parse_forwarded_ip("[2001:db8::1]"), Some(v6));
        assert_eq!(parse_forwarded_ip("unknown"), None);
    }

    #[test]
    fn date_round_trip_is_exact() {
        let now = now();
        assert_eq!(parse_date(&fmt_date(&now)), Some(now));
    }

    #[test]
    fn enc_string_shape() {
        assert!(is_enc_string("1.AAAAAAAAAAAAAAAA.AAAAAAAAAAAAAAAAAAAAAAAA", 100));
        assert!(!is_enc_string("2.AAAAAAAAAAAAAAAA.AAAAAAAAAAAAAAAAAAAAAAAA", 100));
        assert!(!is_enc_string("1.AAAA.AAAAAAAAAAAAAAAAAAAAAAAA", 100));
        assert!(!is_enc_string("plain text", 100));
    }

    #[test]
    fn email_validation() {
        assert!(is_valid_email("me@example.com"));
        assert!(!is_valid_email("me@localhost"));
        assert!(!is_valid_email("no-at-sign"));
        assert!(!is_valid_email("me @example.com"));
    }
}
