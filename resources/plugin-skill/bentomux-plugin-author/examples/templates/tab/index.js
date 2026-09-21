/* Example Plugin — a Bentomux plugin.
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
  ctx.log('Example Plugin activated');

  ctx.ui.command('acme.example.hello', () => {
    ctx.log('hello from Example Plugin');
  });

  ctx.ui.tab('acme.example.tab', host => {
    host.textContent = 'Hello from the Example Plugin tab.';

    /* Return a cleanup and it runs when the tab body is torn down. */
    return () => ctx.log('tab torn down');
  });

  ctx.ui.command('acme.example.open', () => ctx.ui.openTab('acme.example.tab'));

}

/* Optional. Runs when the plugin is disabled or reloaded. Use ctx.dispose()
   for teardown registered during activate. */
export function deactivate() {}
