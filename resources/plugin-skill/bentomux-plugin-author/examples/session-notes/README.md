# Session Notes

The bundled example plugin. It ships enabled with Bentomux and is the worked
example for the plugin platform.

It is deliberately the broadest example: it contributes a **dock button**, a
**modal**, a **widget**, and **two commands**, and uses plugin storage plus a
host event. Anything non-trivial is closer to this than to the single-purpose
templates.

| File | Purpose |
|---|---|
| `plugin.json` | The manifest. |
| `index.js` | The implementation. |
| `bentomux-plugin-sdk.d.ts` | Types for the context API. |

Notes are stored per workspace under the plugin's own data file, which survives
uninstalling the plugin unless you explicitly ask for its removal.
