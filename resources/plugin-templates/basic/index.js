/* {{name}} — a Bentomux plugin.
 *
 * Entry point. Bentomux imports this as an ES module and calls `activate(ctx)`
 * the first time the plugin runs.
 *
 * Everything registered here must be declared in plugin.json first.
 * Registering an id the manifest never declared is an error; declaring an id
 * that is never registered is a warning. That pairing is what keeps the
 * manifest an honest description of what the plugin adds.
 *
 * See bentomux-plugin-sdk.d.ts for the full context API. */

/** @param {import('./bentomux-plugin-sdk').PluginContext} ctx */
export function activate(ctx) {
  ctx.log('{{name}} activated');

  ctx.ui.command('{{id}}.hello', () => {
    ctx.log('hello from {{name}}');
  });

}

/* Optional. Runs when the plugin is disabled or reloaded. Use ctx.dispose()
   for teardown registered during activate. */
export function deactivate() {}
