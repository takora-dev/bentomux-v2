# Example Plugin

A Bentomux plugin scaffold.

| File | Purpose |
|---|---|
| `plugin.json` | The manifest: identity, permissions, and what this plugin contributes. |
| `index.js` | The entry point. Bentomux imports it and calls `activate(ctx)`. |
| `bentomux-plugin-sdk.d.ts` | Types for the context API. |

Edit `plugin.json` to change what the plugin adds, then implement the handlers
in `index.js`. Validate the result with:

    bentomux --plugin-validate .

Reference: `docs/PLUGIN_PLATFORM.md` in the Bentomux repository.
