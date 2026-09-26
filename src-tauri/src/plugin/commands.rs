/* ---------------- plugin platform: IPC commands ----------------
Spec: docs/PLUGIN_PLATFORM.md §8, §11.

Every command here is reachable from the renderer's bridge. Mutating ones
go through AppStateManager::patch_state so the registry is persisted in the
same call that changes it — the plugin list must never be ahead of disk. */

use serde::Serialize;
use tauri::{Manager, State};

use crate::plugin::boot;
use crate::plugin::data;
use crate::plugin::registry;
use crate::plugin::validate::{self, ValidateOptions, ValidationReport};
use crate::plugin::{PluginError, PluginRecord, PluginResult};
use crate::state::{AppState, AppStateManager};

/* ---------------- errors on the wire ---------------- */

/// Commands hand the renderer a structured error rather than a bare string,
/// so Plugin Studio can show a validation report as a list instead of one
/// wall of prose. `code` is stable; `messages` is what a human reads.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<ValidationReport>,
}

impl From<PluginError> for CommandError {
    fn from(e: PluginError) -> Self {
        match e {
            PluginError::Validation(messages) => CommandError {
                code: "validation".into(),
                message: messages.join("; "),
                messages,
                report: None,
            },
            other => CommandError {
                code: match other {
                    PluginError::Io(_) => "io",
                    PluginError::Manifest(_) => "manifest",
                    PluginError::NotFound(_) => "not-found",
                    PluginError::Conflict(_) => "conflict",
                    PluginError::Unsupported(_) => "unsupported",
                    PluginError::Validation(_) => unreachable!(),
                }
                .into(),
                message: other.to_string(),
                messages: vec![],
                report: None,
            },
        }
    }
}

impl From<PluginError> for String {
    fn from(e: PluginError) -> Self {
        serde_json::to_string(&CommandError::from(e))
            .unwrap_or_else(|_| "{\"code\":\"io\",\"message\":\"unknown error\"}".to_string())
    }
}

/* ---------------- paths ---------------- */

/// Resolve the plugin directories from the app data dir, with bundled plugins
/// picked up from the packaged resources when present.
fn plugin_paths(app: &tauri::AppHandle) -> PluginResult<registry::PluginPaths> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| PluginError::Io(format!("app data dir unavailable: {}", e)))?;
    let mut paths = registry::PluginPaths::new(&base);

    /* bundled plugins ship as a resource directory. The dev fallback mirrors
    bridge.rs: the bundler rewrites a leading `..` to `_up_`, and running
    from a source tree has no resource dir at all. */
    if let Ok(dir) = app.path().resolve(
        "../resources/plugin-bundled",
        tauri::path::BaseDirectory::Resource,
    ) {
        if dir.is_dir() {
            paths = paths.with_bundled(dir);
        }
    }
    Ok(paths)
}

/* ---------------- listing ---------------- */

#[tauri::command]
pub fn plugin_list(state: State<'_, AppStateManager>) -> Vec<PluginRecord> {
    state.get_state().plugins
}

/// Native folder picker for the install flow. Separate from `workspace_choose`
/// so the dialog carries plugin wording — the two are different actions and a
/// user should not have to guess which one they are in.
#[tauri::command]
pub fn plugin_choose_folder() -> Option<String> {
    rfd::FileDialog::new()
        .set_title("Choose a plugin folder")
        .pick_folder()
        .map(|p| p.to_string_lossy().to_string())
}

/// Native file picker for a plugin archive. The renderer cannot reach the
/// filesystem, so the path has to come from an OS dialog.
#[tauri::command]
pub fn plugin_choose_zip() -> Option<String> {
    rfd::FileDialog::new()
        .set_title("Choose a plugin package")
        .add_filter("Plugin package", &["zip"])
        .pick_file()
        .map(|p| p.to_string_lossy().to_string())
}

/// Where the Creator wizard should put a new plugin: an empty folder the user
/// picks. Separate from `plugin_choose_folder` because creating a plugin is
/// not installing one, and the wizard must not overwrite an existing project.
#[tauri::command]
pub fn plugin_choose_new_folder() -> Option<String> {
    rfd::FileDialog::new()
        .set_title("Choose an empty folder for the new plugin")
        .pick_folder()
        .map(|p| p.to_string_lossy().to_string())
}

/* ---------------- template scaffolding ---------------- */

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateInfo {
    pub name: String,
    pub contributes: Vec<String>,
}

/// The templates available to the Creator, read from the bundled resources.
/// Read from disk rather than hardcoded so a template added to the repo shows
/// up without a code change.
#[tauri::command]
pub fn plugin_templates(app: tauri::AppHandle) -> Vec<TemplateInfo> {
    let Some(root) = templates_root(&app) else {
        return vec![];
    };
    let Ok(entries) = std::fs::read_dir(&root) else {
        return vec![];
    };

    let mut out: Vec<TemplateInfo> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let manifest_path = e.path().join("plugin.json");
            let text = std::fs::read_to_string(&manifest_path).ok()?;
            let raw: serde_json::Value = serde_json::from_str(&text).ok()?;
            let contributes = raw
                .get("contributes")
                .and_then(|c| c.as_object())
                .map(|m| m.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            Some(TemplateInfo { name, contributes })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Scaffold a plugin from a template into `dest`.
///
/// Refuses a non-empty destination: the wizard must never overwrite a folder
/// the user already has work in, and a half-overwritten plugin is worse than
/// a refused one. Placeholders are substituted here rather than in the
/// renderer so the same code path serves the CLI and the app.
#[tauri::command]
pub fn plugin_scaffold(
    template: String,
    dest: String,
    id: String,
    name: String,
    version: String,
    description: String,
    author: String,
    app: tauri::AppHandle,
) -> Result<ValidationReport, String> {
    use crate::plugin::validate::{validate_dir_with, ValidateOptions};

    let Some(root) = templates_root(&app) else {
        return Err(String::from(PluginError::NotFound(
            "plugin templates are not bundled with this build".into(),
        )));
    };
    /* the template name comes from the renderer, so it is untrusted input:
    resolve it through safe_join rather than joining a raw string */
    let source = crate::plugin::safe_join(&root, &template)
        .ok_or_else(|| String::from(PluginError::NotFound(format!("template `{}`", template))))?;
    if !source.is_dir() {
        return Err(String::from(PluginError::NotFound(format!(
            "template `{}`",
            template
        ))));
    }

    let dest_path = std::path::Path::new(&dest);
    if dest_path.exists() {
        let empty = std::fs::read_dir(dest_path)
            .map(|mut d| d.next().is_none())
            .unwrap_or(false);
        if !empty {
            return Err(String::from(PluginError::Conflict(format!(
                "{} already exists and is not empty",
                dest
            ))));
        }
    }
    std::fs::create_dir_all(dest_path).map_err(|e| String::from(PluginError::Io(e.to_string())))?;

    let values = [
        ("id", id.as_str()),
        ("name", name.as_str()),
        ("version", version.as_str()),
        ("description", description.as_str()),
        ("author", author.as_str()),
    ];

    copy_scaffold(&source, dest_path, &values).map_err(String::from)?;

    /* validate what we just wrote: a scaffold that does not validate is a bug
    in the template, and the user should hear about it immediately rather
    than after writing code against it */
    let report = validate_dir_with(dest_path, &ValidateOptions::default());
    Ok(report)
}

fn copy_scaffold(
    from: &std::path::Path,
    to: &std::path::Path,
    values: &[(&str, &str)],
) -> PluginResult<()> {
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if entry.metadata()?.is_dir() {
            std::fs::create_dir_all(&dst)?;
            copy_scaffold(&src, &dst, values)?;
            continue;
        }
        let text = std::fs::read_to_string(&src)?;
        let mut filled = text;
        for (key, value) in values {
            filled = filled.replace(&format!("{{{{{}}}}}", key), value);
        }
        std::fs::write(&dst, filled)?;
    }
    Ok(())
}

fn templates_root(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    app.path()
        .resolve(
            "../resources/plugin-templates",
            tauri::path::BaseDirectory::Resource,
        )
        .ok()
        .filter(|d| d.is_dir())
}

/* ---------------- authoring skill ---------------- */

const SKILL_ID: &str = "bentomux-plugin-author";

/// Where each agent runtime keeps its skills, as (agent id, display name,
/// directory). Only agents whose skill directory this app already manages are
/// listed: installing into a runtime whose layout we do not understand would
/// be guessing.
fn skill_targets(home: &std::path::Path) -> Vec<(String, String, std::path::PathBuf)> {
    vec![(
        "claude".to_string(),
        "Claude Code".to_string(),
        home.join(".claude").join("skills"),
    )]
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillTarget {
    pub agent_id: String,
    pub agent_name: String,
    pub path: String,
    /// The skill is already installed there.
    pub installed: bool,
}

/// The agent skill directories this build can install into.
#[tauri::command]
pub fn plugin_skill_targets() -> Vec<SkillTarget> {
    let Some(home) = dirs::home_dir() else {
        return vec![];
    };
    skill_targets(&home)
        .into_iter()
        .map(|(agent_id, agent_name, dir)| {
            let installed = dir.join(SKILL_ID).join("SKILL.md").is_file();
            SkillTarget {
                agent_id,
                agent_name,
                path: dir.to_string_lossy().to_string(),
                installed,
            }
        })
        .collect()
}

/// Copy the authoring skill into an agent's skills directory.
///
/// The skill is a folder, not a single file: its reference documents and
/// examples are what make it useful, and the app's own Skills UI reads only
/// `SKILL.md`. That means the skill appears in that UI, but editing it there
/// touches only `SKILL.md` — the references must be edited on disk. That is a
/// deliberate trade: it avoids reshaping the agent-resources subsystem, which
/// is stable and serves every other resource kind.
#[tauri::command]
pub fn plugin_install_skill(agent_id: String, app: tauri::AppHandle) -> Result<String, String> {
    let Some(home) = dirs::home_dir() else {
        return Err(String::from(PluginError::Io("no home directory".into())));
    };
    let Some((_, _, root)) = skill_targets(&home)
        .into_iter()
        .find(|(id, _, _)| *id == agent_id)
    else {
        return Err(String::from(PluginError::NotFound(format!(
            "agent `{}` has no known skills directory",
            agent_id
        ))));
    };

    let source = app
        .path()
        .resolve(
            "../resources/plugin-skill/bentomux-plugin-author",
            tauri::path::BaseDirectory::Resource,
        )
        .ok()
        .filter(|d| d.is_dir())
        .ok_or_else(|| {
            String::from(PluginError::NotFound(
                "the authoring skill is not bundled with this build".into(),
            ))
        })?;

    let dest = root.join(SKILL_ID);
    if dest.exists() {
        std::fs::remove_dir_all(&dest).map_err(|e| String::from(PluginError::Io(e.to_string())))?;
    }
    std::fs::create_dir_all(&dest).map_err(|e| String::from(PluginError::Io(e.to_string())))?;
    copy_scaffold(&source, &dest, &[]).map_err(String::from)?;

    Ok(dest.to_string_lossy().to_string())
}

/// Validate a folder the user picked, without installing it. This is what
/// Plugin Studio calls before showing the "Install" button.
#[tauri::command]
pub fn plugin_validate(path: String, bundled: Option<bool>) -> ValidationReport {
    let opts = ValidateOptions {
        bundled: bundled.unwrap_or(false),
    };
    validate::validate_dir_with(std::path::Path::new(&path), &opts)
}

/// Read a manifest from an installed plugin, for the Studio detail screen.
#[tauri::command]
pub fn plugin_manifest(
    id: String,
    app: tauri::AppHandle,
    state: State<'_, AppStateManager>,
) -> Result<crate::plugin::PluginManifest, String> {
    let paths = plugin_paths(&app).map_err(String::from)?;
    let record = state
        .get_state()
        .plugins
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| String::from(PluginError::NotFound(id.clone())))?;
    let dir = paths.version_dir(&record.id, &record.version);
    validate::read_manifest_unchecked(&dir).map_err(String::from)
}

/* ---------------- install / update ---------------- */

#[tauri::command]
pub fn plugin_install_folder(
    path: String,
    app: tauri::AppHandle,
    state: State<'_, AppStateManager>,
) -> Result<PluginRecord, String> {
    let paths = plugin_paths(&app).map_err(String::from)?;
    let record =
        registry::install_from_folder(&paths, std::path::Path::new(&path)).map_err(String::from)?;

    state.patch_state(|s| {
        /* reinstalling replaces the record rather than duplicating it */
        s.plugins.retain(|r| r.id != record.id);
        s.plugins.push(record.clone());
    });
    Ok(record)
}

#[tauri::command]
pub fn plugin_install_zip(
    path: String,
    app: tauri::AppHandle,
    state: State<'_, AppStateManager>,
) -> Result<PluginRecord, String> {
    let paths = plugin_paths(&app).map_err(String::from)?;
    let record =
        registry::install_from_zip(&paths, std::path::Path::new(&path)).map_err(String::from)?;

    state.patch_state(|s| {
        s.plugins.retain(|r| r.id != record.id);
        s.plugins.push(record.clone());
    });
    Ok(record)
}

/// Install from an HTTPS URL with a pinned digest. The digest is required:
/// without it the URL owner could change what code the app runs.
#[tauri::command]
pub async fn plugin_install_url(
    url: String,
    sha256: String,
    app: tauri::AppHandle,
    state: State<'_, AppStateManager>,
) -> Result<PluginRecord, String> {
    let paths = plugin_paths(&app).map_err(String::from)?;
    /* blocking HTTP on a worker thread: the download must not stall the
    webview's IPC thread while the user watches a spinner */
    let record = tauri::async_runtime::spawn_blocking(move || {
        registry::install_from_url(&paths, &url, &sha256)
    })
    .await
    .map_err(|e| format!("{{\"code\":\"io\",\"message\":\"{}\"}}", e))?
    .map_err(String::from)?;

    state.patch_state(|s| {
        s.plugins.retain(|r| r.id != record.id);
        s.plugins.push(record.clone());
    });
    Ok(record)
}

/* ---------------- enable / disable ---------------- */

#[tauri::command]
pub fn plugin_set_enabled(
    id: String,
    enabled: bool,
    state: State<'_, AppStateManager>,
) -> Result<AppState, String> {
    let mut failure: Option<PluginError> = None;
    let next = state.patch_state(|s| {
        if let Err(e) = registry::set_enabled(&mut s.plugins, &id, enabled) {
            failure = Some(e);
        }
    });
    match failure {
        Some(e) => Err(String::from(e)),
        None => Ok(next),
    }
}

/* ---------------- update / rollback ---------------- */

/// Install a newer version and keep the outgoing one as the rollback target.
#[tauri::command]
pub fn plugin_update(
    path: String,
    app: tauri::AppHandle,
    state: State<'_, AppStateManager>,
) -> Result<PluginRecord, String> {
    let paths = plugin_paths(&app).map_err(String::from)?;
    let fresh =
        registry::install_from_folder(&paths, std::path::Path::new(&path)).map_err(String::from)?;

    let mut out: Option<PluginRecord> = None;
    state.patch_state(|s| {
        if let Some(existing) = s.plugins.iter_mut().find(|r| r.id == fresh.id) {
            registry::commit_update(&paths, existing, &fresh.version, fresh.sha256.clone());
            out = Some(existing.clone());
        }
    });
    out.ok_or_else(|| String::from(PluginError::NotFound(fresh.id)))
}

#[tauri::command]
pub fn plugin_rollback(
    id: String,
    app: tauri::AppHandle,
    state: State<'_, AppStateManager>,
) -> Result<PluginRecord, String> {
    let paths = plugin_paths(&app).map_err(String::from)?;
    let mut out: Option<PluginRecord> = None;
    let mut failure: Option<PluginError> = None;

    state.patch_state(|s| {
        let Some(rec) = s.plugins.iter_mut().find(|r| r.id == id) else {
            failure = Some(PluginError::NotFound(id.clone()));
            return;
        };
        match registry::rollback(&paths, rec) {
            Ok(()) => out = Some(rec.clone()),
            Err(e) => failure = Some(e),
        }
    });

    match (out, failure) {
        (Some(r), _) => Ok(r),
        (None, Some(e)) => Err(String::from(e)),
        (None, None) => Err(String::from(PluginError::NotFound(id))),
    }
}

/* ---------------- uninstall ---------------- */

/// Remove the plugin. `remove_data` is the separate, explicit action: plugin
/// data survives a plain uninstall, because it belongs to the user.
#[tauri::command]
pub fn plugin_uninstall(
    id: String,
    remove_data: bool,
    app: tauri::AppHandle,
    state: State<'_, AppStateManager>,
) -> Result<AppState, String> {
    let paths = plugin_paths(&app).map_err(String::from)?;
    let mut failure: Option<PluginError> = None;

    let next = state.patch_state(|s| {
        if let Err(e) = registry::uninstall(&paths, &mut s.plugins, &id, remove_data) {
            failure = Some(e);
        }
    });

    match failure {
        Some(e) => Err(String::from(e)),
        None => Ok(next),
    }
}

/* ---------------- plugin data ---------------- */

#[tauri::command]
pub fn plugin_data_get(
    id: String,
    key: String,
    app: tauri::AppHandle,
) -> Result<Option<serde_json::Value>, String> {
    let paths = plugin_paths(&app).map_err(String::from)?;
    data::get(&paths, &id, &key).map_err(String::from)
}

#[tauri::command]
pub fn plugin_data_set(
    id: String,
    key: String,
    value: serde_json::Value,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let paths = plugin_paths(&app).map_err(String::from)?;
    data::set(&paths, &id, &key, value).map_err(String::from)
}

#[tauri::command]
pub fn plugin_data_delete(id: String, key: String, app: tauri::AppHandle) -> Result<(), String> {
    let paths = plugin_paths(&app).map_err(String::from)?;
    data::delete(&paths, &id, &key).map_err(String::from)
}

#[tauri::command]
pub fn plugin_data_keys(id: String, app: tauri::AppHandle) -> Result<Vec<String>, String> {
    let paths = plugin_paths(&app).map_err(String::from)?;
    Ok(data::keys(&paths, &id))
}

/* ---------------- boot / safe mode ---------------- */

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeModeStatus {
    pub safe_mode: bool,
    pub attempts: u32,
    pub requested: bool,
}

#[tauri::command]
pub fn plugin_safe_mode(state: State<'_, AppStateManager>) -> SafeModeStatus {
    let s = state.get_state();
    SafeModeStatus {
        safe_mode: s.safe_mode,
        attempts: s.boot_attempts,
        requested: boot::requested_on_cli(),
    }
}

/// The renderer reports a successful first paint. Until this lands, the boot
/// attempt counter keeps climbing and the third attempt runs in safe mode.
#[tauri::command]
pub fn plugin_report_ready(state: State<'_, AppStateManager>) {
    boot::clear_boot(&state);
}

/// Leave safe mode for this session by re-enabling every plugin that safe
/// mode had suppressed, then reloading the window so activation runs again.
#[tauri::command]
pub fn plugin_leave_safe_mode(
    app: tauri::AppHandle,
    state: State<'_, AppStateManager>,
) -> Result<(), String> {
    state.patch_state(|s| {
        s.safe_mode = false;
        s.boot_attempts = 0;
    });
    /* a reload is the honest way back: plugin modules are already imported in
    this realm and cannot be unloaded, so re-activating them in place would
    double-register every contribution */
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.eval("window.location.reload()");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("bentomux-scaffold-{}-{}", tag, std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /* the substitution is what turns a template into a plugin; a missed
    placeholder would ship a manifest with a literal `{{id}}` in it */
    #[test]
    fn copy_scaffold_substitutes_every_placeholder() {
        let base = tmp("subst");
        let from = base.join("tpl");
        let to = base.join("out");
        std::fs::create_dir_all(&from).unwrap();
        /* the caller owns destination creation — plugin_scaffold does it */
        std::fs::create_dir_all(&to).unwrap();
        std::fs::write(
            from.join("plugin.json"),
            r#"{"id":"{{id}}","name":"{{name}}","version":"{{version}}"}"#,
        )
        .unwrap();

        copy_scaffold(
            &from,
            &to,
            &[
                ("id", "acme.x"),
                ("name", "X"),
                ("version", "1.0.0"),
                ("description", ""),
                ("author", ""),
            ],
        )
        .unwrap();

        let out = std::fs::read_to_string(to.join("plugin.json")).unwrap();
        assert_eq!(out, r#"{"id":"acme.x","name":"X","version":"1.0.0"}"#);
        assert!(!out.contains("{{"), "no placeholder may survive: {}", out);

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn copy_scaffold_walks_nested_directories() {
        let base = tmp("nested");
        let from = base.join("tpl");
        std::fs::create_dir_all(from.join("assets")).unwrap();
        std::fs::write(from.join("assets").join("note.txt"), "{{name}}").unwrap();
        let to = base.join("out");

        copy_scaffold(&from, &to, &[("name", "Deep")]).unwrap();
        assert_eq!(
            std::fs::read_to_string(to.join("assets").join("note.txt")).unwrap(),
            "Deep"
        );

        std::fs::remove_dir_all(&base).ok();
    }
}
