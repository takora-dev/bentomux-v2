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

  /* Widgets appear on the welcome page — the app's dashboard. Keep them to a
     summary: the card is a glance, not a workspace. */
  const paint = async host => {
    const count = (await ctx.storage.get('count')) ?? 0;
    host.textContent = 'Counted ' + count + ' time' + (count === 1 ? '' : 's');
  };

  ctx.ui.widget('acme.example.widget', host => {
    void paint(host);
    return () => ctx.log('widget removed');
  });

  /* ctx.storage needs the "storage" permission in plugin.json. */
  ctx.ui.command('acme.example.count', async () => {
    const count = (await ctx.storage.get('count')) ?? 0;
    await ctx.storage.set('count', count + 1);
    ctx.log('count is now', count + 1);
  });

}

/* Optional. Runs when the plugin is disabled or reloaded. Use ctx.dispose()
   for teardown registered during activate. */
export function deactivate() {}
