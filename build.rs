fn main() {
    // The CI release workflow sets RSKYCAM_BUILD to the run number so the
    // binary knows its full 4-part version; local builds are "-dev".
    println!("cargo:rerun-if-env-changed=RSKYCAM_BUILD");
    let pkg = std::env::var("CARGO_PKG_VERSION").expect("cargo sets this");
    let full = match std::env::var("RSKYCAM_BUILD") {
        Ok(n) if !n.is_empty() => format!("{pkg}.{n}"),
        _ => format!("{pkg}-dev"),
    };
    println!("cargo:rustc-env=RSKYCAM_FULL_VERSION={full}");
}
