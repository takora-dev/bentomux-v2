# The Electron→Tauri parity rule is amended for the plugin platform

`MIGRATION_TO_TAURI.md` lists "feature additions" and "architecture changes beyond Electron → Tauri" as non-goals, and `CLAUDE.md` tells contributors to keep parity 1:1. The plugin platform deliberately breaks both: it is a new feature and a new architecture layer. The existing UI and behavior stay intact and the platform is built alongside them, so this is an exception to the parity rule rather than a repeal of it — but the exception has to be written down, or the next contributor will read `CLAUDE.md` and revert the work as drift.

## Consequences

- `CLAUDE.md` must be updated to name the exception and point at the plugin platform docs.
- `PLUGIN_PLATFORM.md` becomes the governing spec for this workstream; the migration spec stops being the last word on scope.
