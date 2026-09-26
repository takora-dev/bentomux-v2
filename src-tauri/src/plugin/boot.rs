/* ---------------- plugin platform: safe mode ----------------
Spec: docs/PLUGIN_PLATFORM.md §8.

Plugin code runs in the app's own realm, so a plugin that throws during
boot can leave the user unable to reach Plugin Studio to disable it. Safe
mode is the exit: the app starts with third-party plugins disabled and
says so, and the user re-enables them one at a time.

Two triggers, deliberately both present:

  --safe-mode            the manual way out, for a user who knows the flag
  boot-attempt counter   the automatic way out

The counter alone would be dangerous: "boot failed" cannot be told apart
from an unrelated crash, so a flaky machine would silently disable the
user's plugins. That is why the flag ships alongside it rather than
instead of it, and why the banner always explains which trigger fired. */

use crate::state::AppStateManager;

pub const SAFE_MODE_FLAG: &str = "--safe-mode";

/// Consecutive boot attempts tolerated before safe mode engages. Two failed
/// boots means the third attempt runs safe — enough to ride out a one-off
/// crash, few enough that a user is not stuck relaunching forever.
pub const MAX_BOOT_ATTEMPTS: u32 = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct BootReport {
    /// Third-party plugins are disabled for this session.
    pub safe_mode: bool,
    /// How many consecutive attempts this boot represents (1 = clean start).
    pub attempts: u32,
    /// Safe mode came from the command line rather than the counter.
    pub requested: bool,
}

/// Called once during setup, before the webview loads. Increments the
/// counter and decides whether this session runs in safe mode.
/// Synchronous write-through: a crash between boot and first paint must not
/// lose the attempt, or the safe-mode counter never engages.
pub fn begin_boot(mgr: &AppStateManager, requested: bool) -> BootReport {
    let mut attempts = 0u32;
    mgr.patch_state_sync(|s| {
        let next = s.prefs.boot_attempts.unwrap_or(0).saturating_add(1);
        s.prefs.boot_attempts = Some(next);
        attempts = next;
    });

    let auto = attempts > MAX_BOOT_ATTEMPTS;
    BootReport {
        safe_mode: requested || auto,
        attempts,
        requested,
    }
}

/// Called when the renderer reports a successful first paint: the boot
/// worked, so the counter starts clean next time.
pub fn clear_boot(mgr: &AppStateManager) {
    mgr.patch_prefs(|p| {
        p.boot_attempts = Some(0);
    });
}

/// Read the flag from the process arguments.
pub fn requested_on_cli() -> bool {
    std::env::args().any(|arg| arg == SAFE_MODE_FLAG)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn mgr(tag: &str) -> (AppStateManager, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("bentomux-boot-{}-{}", tag, std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        (AppStateManager::new(dir.join("bentomux.json")), dir)
    }

    #[test]
    fn first_boot_is_clean() {
        let (m, dir) = mgr("first");
        let r = begin_boot(&m, false);
        assert_eq!(r.attempts, 1);
        assert!(!r.safe_mode);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn two_failed_boots_engage_safe_mode_on_the_third() {
        let (m, dir) = mgr("fail");

        let b1 = begin_boot(&m, false);
        assert!(!b1.safe_mode, "first boot never runs safe");
        let b2 = begin_boot(&m, false);
        assert!(!b2.safe_mode, "second attempt is still a normal boot");
        let b3 = begin_boot(&m, false);
        assert!(b3.safe_mode, "third consecutive attempt runs safe");
        assert_eq!(b3.attempts, 3);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_successful_boot_resets_the_counter() {
        let (m, dir) = mgr("reset");
        begin_boot(&m, false);
        begin_boot(&m, false);
        clear_boot(&m);

        let after = begin_boot(&m, false);
        assert_eq!(after.attempts, 1, "counter starts clean after a good boot");
        assert!(!after.safe_mode);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_flag_forces_safe_mode_without_touching_the_counter_decision() {
        let (m, dir) = mgr("flag");
        let r = begin_boot(&m, true);
        assert!(r.safe_mode);
        assert!(r.requested, "the banner must be able to say why");
        assert_eq!(r.attempts, 1, "a manual safe boot is still a boot attempt");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn counter_survives_a_restart() {
        let (m, dir) = mgr("persist");
        begin_boot(&m, false);
        drop(m);

        let m2 = AppStateManager::new(dir.join("bentomux.json"));
        let r = begin_boot(&m2, false);
        assert_eq!(r.attempts, 2, "the counter is persisted, not in-memory");

        std::fs::remove_dir_all(&dir).ok();
    }
}
