use std::{
    net::IpAddr,
    time::{Duration, Instant},
};

use dashmap::DashMap;

use crate::error::{ApiResult, Error};

const WINDOW: Duration = Duration::from_secs(60);

/// Fixed one-minute window per key (client IP), for login-style endpoints.
pub struct RateLimiter {
    per_minute: u32,
    buckets: DashMap<String, (Instant, u32)>,
}

impl RateLimiter {
    pub fn new(per_minute: u32) -> Self {
        Self {
            per_minute,
            buckets: DashMap::new(),
        }
    }

    /// Count one attempt by `ip` against the `scope` limit.
    pub fn check_ip(&self, scope: &str, ip: &str) -> ApiResult<()> {
        self.check(&format!("{scope}:{}", bucket(ip)))
    }

    pub fn check(&self, key: &str) -> ApiResult<()> {
        if self.per_minute == 0 {
            return Ok(());
        }
        let now = Instant::now();
        let mut bucket = self.buckets.entry(key.to_string()).or_insert((now, 0));
        if now.duration_since(bucket.0) >= WINDOW {
            *bucket = (now, 0);
        }
        bucket.1 += 1;
        if bucket.1 > self.per_minute {
            return Err(Error::RateLimited);
        }
        Ok(())
    }

    pub fn cleanup(&self) {
        let now = Instant::now();
        self.buckets.retain(|_, (start, _)| now.duration_since(*start) < WINDOW);
    }
}

/// The bucket an address counts against. One IPv6 subscriber usually gets a whole /64 and can
/// use any address in it, so IPv6 is limited per /64 network instead of per address.
fn bucket(ip: &str) -> String {
    match ip.parse::<IpAddr>().map(|ip| ip.to_canonical()) {
        Ok(IpAddr::V6(v6)) => {
            let s = v6.segments();
            format!("{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3])
        }
        Ok(v4) => v4.to_string(),
        Err(_) => ip.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_after_limit() {
        let limiter = RateLimiter::new(2);
        assert!(limiter.check("ip").is_ok());
        assert!(limiter.check("ip").is_ok());
        assert!(limiter.check("ip").is_err());
        assert!(limiter.check("other").is_ok());
    }

    #[test]
    fn ipv6_shares_a_bucket_per_64() {
        let limiter = RateLimiter::new(2);
        assert!(limiter.check_ip("auth", "2001:db8:1:2::1").is_ok());
        assert!(limiter.check_ip("auth", "2001:db8:1:2:ffff::9").is_ok());
        assert!(limiter.check_ip("auth", "2001:db8:1:2::abcd").is_err());
        assert!(limiter.check_ip("auth", "2001:db8:1:3::1").is_ok());
        // IPv4 (also when written as an IPv4-mapped IPv6 address) stays per address.
        assert!(limiter.check_ip("auth", "198.51.100.1").is_ok());
        assert!(limiter.check_ip("auth", "::ffff:198.51.100.1").is_ok());
        assert!(limiter.check_ip("auth", "198.51.100.1").is_err());
        assert!(limiter.check_ip("auth", "198.51.100.2").is_ok());
        // Scopes are separate.
        assert!(limiter.check_ip("send", "198.51.100.1").is_ok());
    }
}
