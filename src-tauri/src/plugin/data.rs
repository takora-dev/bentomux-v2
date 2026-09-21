/* ---------------- plugin platform: per-plugin data store ----------------
   Spec: docs/PLUGIN_PLATFORM.md §9.

   `ctx.storage` is one JSON file per plugin under `plugin-data/<id>.json`,
   written with the same temp-then-rename discipline as the app store so a
   crash mid-write cannot truncate a user's data.

   This is the user's data, not the plugin's: uninstalling a plugin keeps its
   file unless the user explicitly asks for removal (registry::uninstall). */

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::{PluginError, PluginResult};
use super::registry::PluginPaths;

/// Soft cap on one plugin's stored values. A plugin that needs more than this
/// is using the wrong storage, and a clear error beats a silent truncation.
pub const MAX_DATA_BYTES: u64 = 1024 * 1024;

type Map = BTreeMap<String, serde_json::Value>;

fn read(path: &Path) -> Map {
    /* a missing or corrupt file reads as empty: plugin data is a convenience,
       never something the app must refuse to start over */
    fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Map>(&t).ok())
        .unwrap_or_default()
}

fn write(path: &Path, map: &Map) -> PluginResult<()> {
    let json = serde_json::to_string_pretty(map)
        .map_err(|e| PluginError::Io(format!("serialize plugin data: {}", e)))?;
    if json.len() as u64 > MAX_DATA_BYTES {
        return Err(PluginError::Conflict(format!(
            "plugin data would be {} bytes, over the {} byte cap",
            json.len(),
            MAX_DATA_BYTES
        )));
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

pub fn get(paths: &PluginPaths, id: &str, key: &str) -> PluginResult<Option<serde_json::Value>> {
    Ok(read(&paths.data_file(id)).get(key).cloned())
}

pub fn set(
    paths: &PluginPaths,
    id: &str,
    key: &str,
    value: serde_json::Value,
) -> PluginResult<()> {
    let path = paths.data_file(id);
    let mut map = read(&path);
    map.insert(key.to_string(), value);
    write(&path, &map)
}

pub fn delete(paths: &PluginPaths, id: &str, key: &str) -> PluginResult<()> {
    let path = paths.data_file(id);
    let mut map = read(&path);
    map.remove(key);
    write(&path, &map)
}

pub fn keys(paths: &PluginPaths, id: &str) -> Vec<String> {
    read(&paths.data_file(id)).keys().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(tag: &str) -> (PluginPaths, std::path::PathBuf) {
        let root = std::env::temp_dir()
            .join(format!("bentomux-pdata-{}-{}", tag, std::process::id()));
        fs::remove_dir_all(&root).ok();
        fs::create_dir_all(&root).unwrap();
        (PluginPaths::new(&root), root)
    }

    #[test]
    fn values_round_trip_across_reloads() {
        let (p, root) = paths("roundtrip");
        set(&p, "acme.t", "count", serde_json::json!(3)).unwrap();
        set(&p, "acme.t", "label", serde_json::json!("hi")).unwrap();

        assert_eq!(get(&p, "acme.t", "count").unwrap(), Some(serde_json::json!(3)));
        assert_eq!(get(&p, "acme.t", "label").unwrap(), Some(serde_json::json!("hi")));
        assert_eq!(keys(&p, "acme.t"), vec!["count".to_string(), "label".to_string()]);

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_key_and_missing_file_are_both_none() {
        let (p, root) = paths("missing");
        assert_eq!(get(&p, "acme.t", "nope").unwrap(), None);
        set(&p, "acme.t", "a", serde_json::json!(1)).unwrap();
        assert_eq!(get(&p, "acme.t", "nope").unwrap(), None);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn delete_removes_only_that_key() {
        let (p, root) = paths("delete");
        set(&p, "acme.t", "a", serde_json::json!(1)).unwrap();
        set(&p, "acme.t", "b", serde_json::json!(2)).unwrap();
        delete(&p, "acme.t", "a").unwrap();

        assert_eq!(get(&p, "acme.t", "a").unwrap(), None);
        assert_eq!(get(&p, "acme.t", "b").unwrap(), Some(serde_json::json!(2)));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn plugins_do_not_see_each_others_data() {
        let (p, root) = paths("isolated");
        set(&p, "acme.a", "k", serde_json::json!("a")).unwrap();
        set(&p, "acme.b", "k", serde_json::json!("b")).unwrap();

        assert_eq!(get(&p, "acme.a", "k").unwrap(), Some(serde_json::json!("a")));
        assert_eq!(get(&p, "acme.b", "k").unwrap(), Some(serde_json::json!("b")));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn corrupt_file_reads_as_empty_instead_of_failing() {
        let (p, root) = paths("corrupt");
        fs::create_dir_all(&p.data_root).unwrap();
        fs::write(p.data_file("acme.t"), "{ not json").unwrap();

        assert_eq!(get(&p, "acme.t", "k").unwrap(), None);
        /* and a write repairs it */
        set(&p, "acme.t", "k", serde_json::json!(1)).unwrap();
        assert_eq!(get(&p, "acme.t", "k").unwrap(), Some(serde_json::json!(1)));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn oversized_value_is_refused_rather_than_truncated() {
        let (p, root) = paths("big");
        let big = "x".repeat((MAX_DATA_BYTES as usize) + 1024);
        let err = set(&p, "acme.t", "k", serde_json::json!(big));
        assert!(matches!(err, Err(PluginError::Conflict(_))));
        assert_eq!(get(&p, "acme.t", "k").unwrap(), None, "nothing partial written");

        fs::remove_dir_all(&root).ok();
    }
}
