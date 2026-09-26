/* ---------------- agent config adapters ----------------
Rust port of src/main/agents/. index.rs owns the registry and the
agents_info / config-view / model-settings surfaces; claude_code.rs and
pi.rs are the two real resource adapters, generic.rs the seven simple
config-file adapters, util.rs the shared file/TOML/ENV/frontmatter
helpers, types.rs the adapter contract. */

pub mod claude_code;
pub mod generic;
pub mod index;
pub mod pi;
pub mod resources;
pub mod types;
pub mod util;
