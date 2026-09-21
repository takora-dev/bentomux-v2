# Plugins run in-realm as ES modules; permissions are a contract, not a security boundary

Plugin code is loaded into the main window's JavaScript realm as an ES module over a custom `plugin://` scheme, and the host hands it a `ctx` object scoped to the permissions its manifest declared. We chose this over sandboxed iframes, a separate webview per plugin, and declarative-only plugins because every isolation mechanism available in this stack makes inline contributions — a real topbar button, a real sidebar row — impossible or visually wrong, and because this is a local desktop tool whose plugins are installed deliberately by the person using them.

## Considered Options

- **Sandboxed iframe per plugin** — real isolation, but contributions degrade to embedded frames and plugins lose the app's styling and DOM helpers.
- **Separate webview per plugin** — real process isolation, but every contribution becomes an OS window; nothing can sit inside the sidebar.
- **Declarative-only, no executable code** — safe, but cannot express commands, services, or real logic.

## Consequences

- The permission list is enforced by the host's `ctx` facade, not by the runtime. Same-realm code shares the page's globals, including the bridge the runtime injects for IPC, so a plugin can reach anything the page can reach. Permissions exist so a plugin's intent is legible and checkable, not to contain a hostile plugin.
- `deactivate()` is part of the contract, but ES modules cannot be unloaded: reloading a plugin re-imports it under a cache-busting URL, leaking the previous module's scope. Honest teardown limits the leak to memory, not behavior.
- A plugin's uncaught error can break the shell. The host wraps activation and service startup in try/catch and marks the plugin errored, but it cannot contain errors thrown later from plugin-owned callbacks.

## What we gave up

Verified in `tauri-2.11.5`: the IPC bridge is injected as a main-frame-only initialization script (`src/manager/webview.rs:166-195`), and it reaches the backend through `window.__TAURI_INTERNALS__.invoke(...)` (`src/ipc/channel.rs`). So a sandboxed iframe would in fact be *more* isolated than same-realm loading — it could not reach the bridge at all. That is the real cost of this decision, stated plainly: we traded isolation for the ability to render inline contributions that look and behave like the app's own.

If a future requirement demands executing genuinely untrusted plugin code, the iframe path is the one to revisit — and it would require rethinking every inline contribution point, not just the loader.
