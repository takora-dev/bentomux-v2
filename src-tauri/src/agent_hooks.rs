/* ---------------- managed Claude Code hooks ----------------
Rust port of src/main/agent-hooks.ts. Installs/removes Bentomux's hook
entries in ~/.claude/settings.json so Claude Code forwards
PermissionRequest / Notification / Stop events to the bridge. Ownership
marker: any command entry whose args reference bentomux-hook.cjs.
Foreign hooks are never touched; install is idempotent (stale paths from
a previous install are replaced). Deliberately free of any tauri/app
dependency so the installer can be unit-tested directly against a temp
settings file. */

use std::collections::HashMap;

use crate::agents::util::{read_json, write_json};
use crate::runtime::AgentHooksStatus;

pub const MARKER: &str = "bentomux-hook.cjs";
/* entries installed before the Bentomux rebrand reference takora-hook.cjs */
const LEGACY_MARKER: &str = "takora-hook.cjs";

/* [event, seconds the hook may stay blocked] — PermissionRequest waits
for the user's decision and the documented default timeout (600s) is
far too short. It is the only managed event: approvals surface in the
overlay window alone, so Notification/Stop hooks would be overhead. */
const MANAGED_EVENTS: &[(&str, u64)] = &[("PermissionRequest", 86400)];

pub fn default_settings_path() -> String {
    dirs::home_dir()
        .map(|p| {
            p.join(".claude")
                .join("settings.json")
                .to_string_lossy()
                .into_owned()
        })
        .unwrap_or_default()
}

fn is_ours(h: &serde_json::Value) -> bool {
    h.get("type").and_then(serde_json::Value::as_str) == Some("command")
        && h.get("command").and_then(serde_json::Value::as_str) == Some("node")
        && h.get("args")
            .and_then(|a| a.as_array())
            .map(|args| {
                args.iter().any(|a| {
                    a.as_str()
                        .map(|s| s.contains(MARKER) || s.contains(LEGACY_MARKER))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
}

fn group_is_ours(g: &serde_json::Value) -> bool {
    g.get("hooks")
        .and_then(|hooks| hooks.as_array())
        .map(|hs| hs.iter().any(is_ours))
        .unwrap_or(false)
}

/* distinguish a missing file (fine — install creates it) from one that
exists but is not valid JSON (never silently overwrite user settings) */
fn load_settings(settings_path: &str) -> (serde_json::Value, Option<String>) {
    if !std::path::Path::new(settings_path).exists() {
        return (serde_json::json!({}), None);
    }
    match read_json::<serde_json::Value>(settings_path) {
        Some(v) if v.is_object() => (v, None),
        _ => (
            serde_json::json!({}),
            Some(format!("Invalid JSON in {settings_path}")),
        ),
    }
}

fn status(settings_path: &str, installed: bool, error: Option<String>) -> AgentHooksStatus {
    AgentHooksStatus {
        installed,
        settings_path: settings_path.to_string(),
        error,
    }
}

pub fn read_hook_status(settings_path: &str) -> AgentHooksStatus {
    let (obj, error) = load_settings(settings_path);
    if error.is_some() {
        return status(settings_path, false, error);
    }
    let installed = MANAGED_EVENTS.iter().all(|(event, _)| {
        obj.get("hooks")
            .and_then(|h| h.get(event))
            .and_then(|groups| groups.as_array())
            .map(|groups| groups.iter().any(group_is_ours))
            .unwrap_or(false)
    });
    status(settings_path, installed, None)
}

pub fn install_hooks(settings_path: &str, script_path: &str) -> AgentHooksStatus {
    let (obj, error) = load_settings(settings_path);
    if let Some(e) = error {
        return status(settings_path, false, Some(e));
    }
    let mut hooks: serde_json::Map<String, serde_json::Value> = obj
        .get("hooks")
        .and_then(|h| h.as_object())
        .cloned()
        .unwrap_or_default();
    for (event, timeout) in MANAGED_EVENTS {
        /* drop stale paths from a previous install, then add ours fresh */
        let mut groups: Vec<serde_json::Value> = hooks
            .get(*event)
            .and_then(|g| g.as_array())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|g| !group_is_ours(g))
            .collect();
        groups.push(serde_json::json!({ "hooks": [{
            "type": "command",
            "command": "node",
            "args": [script_path],
            "timeout": timeout,
        }] }));
        hooks.insert((*event).to_string(), serde_json::Value::Array(groups));
    }
    let mut next = obj.clone();
    if let Some(o) = next.as_object_mut() {
        o.insert("hooks".to_string(), serde_json::Value::Object(hooks));
    }
    write_json(settings_path, &next);
    read_hook_status(settings_path)
}

pub fn uninstall_hooks(settings_path: &str) -> AgentHooksStatus {
    let (obj, error) = load_settings(settings_path);
    if error.is_some() {
        return read_hook_status(settings_path);
    }
    let Some(hooks) = obj.get("hooks").and_then(|h| h.as_object()).cloned() else {
        return read_hook_status(settings_path);
    };
    let mut kept: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
    for (event, groups) in hooks {
        let filtered: Vec<serde_json::Value> = groups
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|g| !group_is_ours(g))
            .collect();
        if !filtered.is_empty() {
            kept.insert(event, serde_json::Value::Array(filtered));
        }
    }
    let mut next = obj.clone();
    if let Some(o) = next.as_object_mut() {
        o.insert("hooks".to_string(), serde_json::Value::Object(kept));
    }
    write_json(settings_path, &next);
    read_hook_status(settings_path)
}

/* keyed test helpers — expose the raw mutated hooks map so tests need not
fabricate the whole AgentHooksStatus */
pub fn hook_events_present(settings_path: &str) -> HashMap<String, bool> {
    let (obj, _) = load_settings(settings_path);
    let mut out = HashMap::new();
    for (event, _) in MANAGED_EVENTS {
        let present = obj
            .get("hooks")
            .and_then(|h| h.get(*event))
            .and_then(|g| g.as_array())
            .map(|gs| gs.iter().any(group_is_ours))
            .unwrap_or(false);
        out.insert((*event).to_string(), present);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_settings(suffix: &str) -> String {
        std::env::temp_dir()
            .join(format!(
                "bentomux-hook-test-{}-{suffix}.json",
                std::process::id()
            ))
            .to_string_lossy()
            .into_owned()
    }

    fn clean(p: &str) {
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn missing_file_reports_not_installed_no_error() {
        let p = temp_settings("missing");
        clean(&p);
        let st = read_hook_status(&p);
        assert!(!st.installed);
        assert_eq!(st.error, None);
        clean(&p);
    }

    #[test]
    fn install_then_uninstall_round_trip() {
        let p = temp_settings("rt");
        clean(&p);
        let st = install_hooks(&p, "/tmp/bentomux-hook.cjs");
        assert!(st.installed, "hooks should install: {st:?}");
        assert!(st.error.is_none());
        assert!(hook_events_present(&p)
            .get("PermissionRequest")
            .copied()
            .unwrap_or(false));

        /* install again is idempotent — exactly one of our groups */
        install_hooks(&p, "/tmp/bentomux-hook.cjs");
        let groups = read_json::<serde_json::Value>(&p)
            .and_then(|v| v.get("hooks").cloned())
            .and_then(|h| h.get("PermissionRequest").cloned())
            .and_then(|g| g.as_array().cloned())
            .unwrap_or_default();
        assert_eq!(groups.iter().filter(|g| group_is_ours(g)).count(), 1);

        let un = uninstall_hooks(&p);
        assert!(!un.installed);
        assert!(!hook_events_present(&p)
            .get("PermissionRequest")
            .copied()
            .unwrap_or(false));
        clean(&p);
    }

    #[test]
    fn install_preserves_foreign_hooks() {
        let p = temp_settings("foreign");
        clean(&p);
        let foreign = serde_json::json!({
            "hooks": { "PermissionRequest": [{ "hooks": [{ "type": "command", "command": "node", "args": ["/usr/bin/custom-hook.cjs"], "timeout": 10 }] }] }
        });
        write_json(&p, &foreign);
        install_hooks(&p, "/tmp/bentomux-hook.cjs");
        let groups = read_json::<serde_json::Value>(&p)
            .and_then(|v| v.get("hooks").cloned())
            .and_then(|h| h.get("PermissionRequest").cloned())
            .and_then(|g| g.as_array().cloned())
            .unwrap_or_default();
        /* foreign still present, plus our one = 2 */
        assert_eq!(groups.len(), 2);
        clean(&p);
    }

    #[test]
    fn invalid_json_reports_error_without_overwrite() {
        let p = temp_settings("bad");
        std::fs::write(&p, "{ invalid json").unwrap();
        let st = install_hooks(&p, "/tmp/x.cjs");
        assert!(!st.installed);
        assert!(st.error.is_some());
        let raw = std::fs::read_to_string(&p).unwrap();
        assert_eq!(raw, "{ invalid json"); /* not clobbered */
        clean(&p);
    }
}
