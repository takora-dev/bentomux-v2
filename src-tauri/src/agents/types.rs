/* ---------------- agent adapter contract ----------------
Rust port of src/main/agents/types.ts. This is the surface each agent
runtime adapter implements; the registry in index.rs holds one per agent. */

use std::collections::HashMap;

use crate::state::{Capabilities, ResourceKind, WorkspaceRec};

/* ---- resource definitions (what lands in agent files) ---- */
#[derive(Clone, Debug)]
pub struct MemoryDef {
    pub scope: MemoryScope,
    pub workspace_path: Option<String>,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemoryScope {
    Global,
    Project,
}

#[derive(Clone, Debug)]
pub struct SkillDef {
    pub description: String,
    pub instructions: String,
}

#[derive(Clone, Debug)]
pub struct McpDef {
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

#[derive(Clone, Debug)]
pub enum ResourceDef {
    Memory(MemoryDef),
    Skill(SkillDef),
    Mcp(McpDef),
}

/* one managed resource as read from the agent's config */
#[derive(Clone, Debug)]
pub struct ListedEntry {
    pub def: ResourceDef,
    pub updated_at: u64,
    pub name: Option<String>,
}

pub type ListMap = HashMap<String, ListedEntry>;

/* injected into each adapter at construction */
pub struct AdapterCtx {
    pub home: String,
    pub workspaces: Vec<WorkspaceRec>,
}

/* the Model-tab surface an adapter may expose. Fields outside `fields` are
not rendered; an empty-string patch value clears the field, except apiKey
which arrives only when the user typed a new one (null = clear). */
pub struct ModelSettingsCap {
    pub fields: Vec<ModelField>,
    /* fixed choices for the format field; empty = free-text input */
    pub formats: Vec<FormatChoice>,
    /* well-known model ids offered as suggestions (custom ids always allowed) */
    pub suggestions: Vec<String>,
    /* human-readable file(s) the Apply button writes to */
    pub write_target: String,
    pub get: Box<dyn Fn() -> ModelSettingsView + Send + Sync>,
    pub set: Box<dyn Fn(ModelSettingsPatch) + Send + Sync>,
}

pub trait AgentAdapter: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn capabilities(&self) -> Capabilities;
    fn detect(&self) -> bool;
    fn list(&self, kind: ResourceKind) -> ListMap;
    fn write(&self, kind: ResourceKind, id: String, name: String, def: ResourceDef);
    fn remove(&self, kind: ResourceKind, id: String, def: Option<&ResourceDef>);
    /* native enable/disable in the agent's own config (e.g. Claude
    skillOverrides); when absent the orchestrator falls back to the shadow
    store. has_native_toggle defaults false and is overridden by adapters
    that implement native_toggle. */
    fn has_native_toggle(&self) -> bool {
        false
    }
    fn native_toggle(&self, kind: ResourceKind, id: String, on: bool) {
        let _ = (kind, id, on);
    }
    fn is_natively_off(&self, kind: ResourceKind, id: String) -> bool {
        let _ = (kind, id);
        false
    }
    /* editable model settings; None when the agent exposes none */
    fn model_settings(&self) -> Option<&ModelSettingsCap>;
    fn config_path(&self) -> Option<String>;
}

/* re-export the state-adjacent contract for concise imports */
pub use crate::runtime::{FormatChoice, ModelField, ModelSettingsPatch, ModelSettingsView};
