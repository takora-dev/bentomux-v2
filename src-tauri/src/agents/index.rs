/* ---------------- agent adapters: contract + registry ----------------
Rust port of src/main/agents/index.ts. Owns the adapter registry and the
three surfaces consumed by the runtime commands: agents info, per-agent
config view, and model-settings apply. Workspaces are passed in per call
so the config paths track live state without a callback indirection. */

use crate::state::{AgentInfo, ResourceKind, WorkspaceRec};

use super::claude_code::ClaudeCodeAdapter;
use super::generic;
use super::pi::PiAdapter;
use super::types::{AdapterCtx, AgentAdapter, ListedEntry};
use crate::runtime::{AgentConfigView, ModelSettingsPatch, ModelSettingsView, ResourceCounts};

fn ctx_for(workspaces: &[WorkspaceRec]) -> AdapterCtx {
    let home = dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    AdapterCtx {
        home,
        workspaces: workspaces.to_vec(),
    }
}

/* a fresh registry per call — cheap to build and keeps the detect()/config
paths honest to the live workspace list (Electron read them via a closure) */
pub fn all(workspaces: &[WorkspaceRec]) -> Vec<Box<dyn AgentAdapter>> {
    let ctx = ctx_for(workspaces);
    let mut v: Vec<Box<dyn AgentAdapter>> = vec![
        Box::new(ClaudeCodeAdapter::new(&ctx)),
        Box::new(PiAdapter::new(&ctx)),
    ];
    for a in [
        generic::qwen_adapter(&ctx),
        generic::codex_adapter(&ctx),
        generic::opencode_adapter(&ctx),
        generic::gemini_adapter(&ctx),
        generic::cursor_adapter(&ctx),
        generic::kilo_adapter(&ctx),
        generic::qwenpaw_adapter(&ctx),
    ] {
        v.push(a);
    }
    v
}

fn adapter_for<'a>(
    id: &str,
    adapters: &'a [Box<dyn AgentAdapter>],
) -> Option<&'a dyn AgentAdapter> {
    adapters.iter().find(|a| a.id() == id).map(|b| b.as_ref())
}

pub fn find_adapter<'a>(
    id: &str,
    adapters: &'a [Box<dyn AgentAdapter>],
) -> Option<&'a dyn AgentAdapter> {
    adapter_for(id, adapters)
}

fn safe_model_settings(a: &dyn AgentAdapter) -> Option<ModelSettingsView> {
    a.model_settings().map(|cap| (cap.get)())
}

fn safe_config_path(a: &dyn AgentAdapter) -> Option<String> {
    a.config_path()
}

fn safe_list(a: &dyn AgentAdapter, kind: ResourceKind) -> usize {
    if !capability(a, kind.clone()) || !a.detect() {
        return 0;
    }
    a.list(kind).len()
}

fn capability(a: &dyn AgentAdapter, kind: ResourceKind) -> bool {
    let c = a.capabilities();
    match kind {
        ResourceKind::Memory => c.memory,
        ResourceKind::Skills => c.skills,
        ResourceKind::Mcp => c.mcp,
    }
}

pub fn agents_info(workspaces: &[WorkspaceRec]) -> Vec<AgentInfo> {
    all(workspaces)
        .iter()
        .map(|a| {
            let settings = safe_model_settings(a.as_ref());
            AgentInfo {
                id: a.id().to_string(),
                name: a.name().to_string(),
                detected: a.detect(),
                capabilities: a.capabilities(),
                current_model: settings.as_ref().and_then(|s| {
                    if s.model.is_empty() {
                        None
                    } else {
                        Some(s.model.clone())
                    }
                }),
                config_path: safe_config_path(a.as_ref()),
            }
        })
        .collect()
}

pub fn agent_config_view(workspaces: &[WorkspaceRec], agent_id: &str) -> Option<AgentConfigView> {
    let adapters = all(workspaces);
    let a = adapter_for(agent_id, &adapters)?;
    let cap_opt = a.model_settings();
    let settings = cap_opt.map(|c| (c.get)());
    let model_fields: Vec<_> = cap_opt.map(|c| c.fields.clone()).unwrap_or_default();
    let model_formats: Vec<_> = cap_opt.map(|c| c.formats.clone()).unwrap_or_default();
    let model_suggestions: Vec<_> = cap_opt.map(|c| c.suggestions.clone()).unwrap_or_default();
    let model_write_target: Option<String> = cap_opt.map(|c| c.write_target.clone());
    Some(AgentConfigView {
        id: a.id().to_string(),
        name: a.name().to_string(),
        detected: a.detect(),
        capabilities: a.capabilities(),
        current_model: settings.as_ref().and_then(|s| {
            if s.model.is_empty() {
                None
            } else {
                Some(s.model.clone())
            }
        }),
        model_settings: settings,
        model_fields,
        model_formats,
        model_suggestions,
        model_write_target,
        config_path: safe_config_path(a),
        counts: ResourceCounts {
            memory: safe_list(a, ResourceKind::Memory) as u32,
            skills: safe_list(a, ResourceKind::Skills) as u32,
            mcp: safe_list(a, ResourceKind::Mcp) as u32,
        },
    })
}

/* only fields the agent's capability declares pass through; everything
else in the patch is ignored */
pub fn set_agent_model_settings(
    workspaces: &[WorkspaceRec],
    agent_id: &str,
    patch: ModelSettingsPatch,
) -> Result<AgentConfigView, String> {
    let adapters = all(workspaces);
    let a = adapter_for(agent_id, &adapters).ok_or_else(|| format!("Unknown agent: {agent_id}"))?;
    let cap = a
        .model_settings()
        .ok_or_else(|| format!("{} has no model settings", a.name()))?;
    let filtered = filter_patch(&cap.fields, patch);
    (cap.set)(filtered);
    agent_config_view(workspaces, agent_id).ok_or_else(|| format!("Agent disappeared: {agent_id}"))
}

fn filter_patch(
    fields: &[crate::runtime::ModelField],
    patch: ModelSettingsPatch,
) -> ModelSettingsPatch {
    use crate::runtime::ModelField::*;
    let mut out = ModelSettingsPatch::default();
    let has = |f: crate::runtime::ModelField| fields.contains(&f);
    if has(Model) {
        out.model = patch.model;
    }
    if has(BaseUrl) {
        out.base_url = patch.base_url;
    }
    if has(Context) {
        out.context = patch.context;
    }
    if has(ApiKey) {
        out.api_key = patch.api_key;
    }
    if has(Format) {
        out.format = patch.format;
    }
    out
}

/* resource CRUD surfaces: list/write/remove/toggle route to the owning
adapter and the shadow store. Exposed for the Phase-gated `res:*`
commands once those register. */

#[allow(unused)]
fn list_resources(
    workspaces: &[WorkspaceRec],
    _agent_id: &str,
    kind: ResourceKind,
) -> std::collections::HashMap<String, ListedEntry> {
    let mut out = std::collections::HashMap::new();
    for a in all(workspaces) {
        if capability(a.as_ref(), kind.clone()) && a.detect() {
            for (id, e) in a.list(kind.clone()) {
                out.insert(id, e);
            }
        }
    }
    out
}
