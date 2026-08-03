//! Build version and release-version comparison.
//!
//! CI builds embed `X.Y.Z.<run_number>` (via RSKYCAM_BUILD in build.rs);
//! local builds embed `X.Y.Z-dev`. GitHub release tags are `vX.Y.Z.N`.

/// Full version baked in at compile time.
pub fn full() -> &'static str {
    env!("RSKYCAM_FULL_VERSION")
}

/// Parse `v?X.Y.Z` or `v?X.Y.Z.N` into 4 numbers; a missing 4th part
/// sorts below any explicit build number (legacy `v0.5.0` < `v0.5.0.1`).
#[allow(dead_code)]
fn parse(v: &str) -> Option<[i64; 4]> {
    let v = v.strip_prefix('v').unwrap_or(v);
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() != 3 && parts.len() != 4 {
        return None;
    }
    let mut nums = [0i64, 0, 0, -1];
    for (i, p) in parts.iter().enumerate() {
        nums[i] = p.parse().ok()?;
    }
    Some(nums)
}

/// True when `latest` is a parseable release version strictly newer than
/// `current`. An unparseable current (a `-dev` build) counts as older
/// than any release, so dev Pis are offered updates; an unparseable
/// latest never triggers an update.
#[allow(dead_code)]
pub fn update_available(current: &str, latest: &str) -> bool {
    match (parse(current), parse(latest)) {
        (_, None) => false,
        (None, Some(_)) => true,
        (Some(c), Some(l)) => l > c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_version_embeds_the_cargo_package_version() {
        assert!(full().starts_with(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn version_comparison_table() {
        // (current, latest, expected)
        let cases = [
            ("0.5.0.6", "v0.5.0.7", true),
            ("0.5.0.7", "v0.5.0.7", false),
            ("0.5.0.8", "v0.5.0.7", false),
            ("0.5.0.6", "v0.6.0.1", true),
            ("v0.5.0", "v0.5.0.1", true), // legacy 3-part tag is older
            ("0.5.0-dev", "v0.5.0.1", true), // dev build takes any release
            ("0.5.0-dev", "garbage", false), // unparseable latest: never
            ("0.5.0.6", "v0.5.0", false), // downgrade to legacy: no
        ];
        for (current, latest, expected) in cases {
            assert_eq!(
                update_available(current, latest),
                expected,
                "current={current} latest={latest}"
            );
        }
    }
}
