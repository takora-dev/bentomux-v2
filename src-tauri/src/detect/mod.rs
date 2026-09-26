/* ---------------- agent detection (screen manifests + headless screen) ----------------
Rust port of src/main/detect/. rules.rs is the herdr-style rule engine,
manifests.rs holds per-agent rule tables, screen.rs caches snapshots emitted
by the single authoritative VT parser in the persistent PTY host. */

pub mod manifests;
pub mod rules;
pub mod screen;
