# Plugin code is served over a custom `plugin://` scheme, not eval, blob, or data URLs

Production CSP is `script-src 'self'` with no `'unsafe-eval'` and no `blob:`, so loading plugin code requires widening the policy. The four ways to do it are not equivalent: `eval`/`new Function` needs `'unsafe-eval'` (the widest and hardest to audit), blob URLs need `blob:` in `script-src` (no stable identity per plugin, awkward to cache-bust), and data URLs need `data:` (same problems plus size limits). A custom scheme gives every plugin a real, inspectable URL space, so the loader can cache-bust by version, the host can route per-plugin paths, and a reviewer can see exactly which origin plugin code comes from.

## Consequences

- The CSP must name the plugin scheme in `script-src`, `style-src`, and `img-src`. The URL form differs by platform — `plugin://localhost/…` on macOS and Linux, `http://plugin.localhost/…` on Windows — so both forms go into the policy. Verify the exact per-platform form against the running app when implementing.
- Serving files means the host owns path resolution and must refuse any path outside a plugin's own root.
- The scheme becomes part of the platform's public surface: changing it later breaks every plugin's asset URLs.
