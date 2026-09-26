fn main() {
    tauri_build::build();

    /* Windows test harnesses import comctl32!TaskDialogIndirect (through rfd)
    which comctl32 v5 does not export — tauri-build embeds the
    Common-Controls-v6 manifest into the app binary only, so test binaries
    die at load with STATUS_ENTRYPOINT_NOT_FOUND. The manifest link args
    are gated behind BENTOMUX_EMBED_TEST_MANIFEST (set by
    scripts/test-backend.mjs) because an always-on /MANIFEST:EMBED would
    duplicate the manifest resource tauri-build already embeds into bins
    (CVT1100), and cargo cannot scope link args to test targets only.
    Run with `npm run test:backend`. */
    if std::env::var_os("BENTOMUX_EMBED_TEST_MANIFEST").is_some()
        && std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("tests.manifest");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
    println!("cargo:rerun-if-env-changed=BENTOMUX_EMBED_TEST_MANIFEST");
    println!(
        "cargo:rerun-if-changed={}",
        std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("tests.manifest")
            .display()
    );
}
