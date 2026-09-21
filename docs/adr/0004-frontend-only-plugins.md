# Plugins cannot add backend commands; they compose an allowlisted set

Plugins run in the frontend and cannot register new Tauri commands. To reach the backend they call a generic passthrough gated by the permissions their manifest declared, restricted to commands the host has allowlisted. We chose this over letting plugins ship Rust (impossible without shipping a compiler and a trust root) and over growing one bespoke command per plugin (which would make the app's command surface a function of what users happened to install).

## Consequences

- The allowlist is a curated list in the host, and it is a review surface: adding a command to it grants every installed plugin the ability to call it.
- The AI authoring skill must state this limit plainly, because "add a backend feature" is the most likely thing a user asks for and the most likely thing an agent would fake.
- Out-of-process sidecar plugins (the `pty_host` pattern — same binary, different flag, JSON-line protocol) are the deferred escape hatch for work that must outlive the window or run in another language.
