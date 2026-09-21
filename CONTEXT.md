# Bentomux

Bentomux is a calm desktop for AI agent runtime workspaces. This glossary fixes the vocabulary of the plugin platform — the words that mean something specific here, and the words we deliberately do not use.

## Language

### Plugin platform

**Plugin**:
An installed, versioned bundle of a manifest and code that adds contributions to Bentomux.
_Avoid_: extension, add-on, mod

**Manifest**:
The `plugin.json` file at a plugin's root, declaring its identity, permissions, and contributions.
_Avoid_: descriptor, package file, plugin config

**Contribution**:
One thing a plugin adds to a host surface — a topbar button, a tab, a command.
_Avoid_: hook, extension point

**Contribution Point**:
A named host surface a plugin may contribute to. The v1 set is fixed: topbar button, sidebar item, sidebar dock item, modal, tab, widget, command, service, settings section.
_Avoid_: slot, mount point, extension point

**Bundled Plugin**:
A plugin shipped inside the app itself. Bundled plugins use the reserved `bentomux.*` id prefix.
_Avoid_: built-in, core plugin, first-party plugin

**Plugin Registry**:
The persisted record of which plugins are installed, at which version, from which source, and whether each is enabled.
_Avoid_: plugin store, marketplace, plugin list

**Plugin Data**:
Per-plugin key–value storage owned by the plugin and preserved when the plugin is uninstalled.
_Avoid_: plugin state, plugin storage (unqualified)

### Authoring

**Plugin SDK**:
The TypeScript types and documentation a plugin author writes against.
_Avoid_: plugin API, plugin library

**Plugin Context** (`ctx`):
The runtime object handed to a plugin on activation, carrying only the namespaces its manifest declared.
_Avoid_: sandbox, plugin scope, plugin environment

**Permission**:
A namespace a plugin declares in its manifest before the host will hand it over in `ctx`.
_Avoid_: capability (reserved for agent resources), scope, grant

**Activation**:
Loading a plugin's code and calling its `activate(ctx)` entry point.
_Avoid_: start, boot, load, init

**Plugin Skill**:
The `bentomux-plugin-author` skill that teaches an agent to author plugins.
_Avoid_: AI Agent Skill, plugin agent skill

**Plugin Studio**:
The in-app surface for creating, importing, and managing plugins. It contains the Creator (template scaffolding).
_Avoid_: plugin manager, plugin IDE

**Validator**:
The component that checks a plugin folder statically and reports errors without executing it.
_Avoid_: linter, checker, verifier

### Existing vocabulary — do not overload

**Agent Skill**:
An instruction bundle managed for an AI agent runtime (Claude Code, pi, …) under that agent's own skills directory. Distinct from Plugin Skill.
_Avoid_: skill (unqualified)

**Capability**:
The set of resource kinds an agent runtime supports (memory, skills, MCP). Not a plugin concept.
_Avoid_: using "capability" for plugin permissions

**Resource**:
A memory, skill, or MCP entry managed for an agent runtime.
_Avoid_: using "resource" for plugin files or plugin data
