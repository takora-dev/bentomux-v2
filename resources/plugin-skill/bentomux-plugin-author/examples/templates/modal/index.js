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

  /* A modal renders into the app's own modal root, so it gets the same
     overlay, focus handling, and Escape behaviour as every built-in dialog. */
  ctx.ui.modal('acme.example.dialog', host => {
    host.textContent = 'Hello from Example Plugin.';

    /* Return a cleanup and it runs when the dialog closes. */
    return () => ctx.log('dialog closed');
  });

  ctx.ui.command('acme.example.open', () => ctx.ui.openModal('acme.example.dialog'));

}

/* Optional. Runs when the plugin is disabled or reloaded. Use ctx.dispose()
   for teardown registered during activate. */
export function deactivate() {}
