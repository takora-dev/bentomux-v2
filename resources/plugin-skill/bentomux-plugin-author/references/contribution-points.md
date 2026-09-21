# Contribution points

What a plugin can add, and what each one means in the app.

| Kind | Where it appears | What the plugin registers |
|---|---|---|
| `commands` | The command palette (Ctrl/Cmd+K) | `ctx.ui.command(id, handler)` |
| `topbar` | Titlebar, right side | fires its `command` |
| `sidebar` | Sidebar, below the workspace list | fires its `command` |
| `dock` | Sidebar bottom dock, beside Settings | fires its `command` |
| `tabs` | Tab strip, grouped under the plugin's name | `ctx.ui.tab(id, render)` |
| `modals` | The app's modal root | `ctx.ui.modal(id, render)` |
| `widgets` | Welcome page card grid | `ctx.ui.widget(id, render)` |
| `settings` | Settings modal, after the built-in sections | `ctx.ui.settingsSection(id, build)` |
| `services` | Headless, runs while the app is open | `ctx.ui.service(id, run)` |

## Activation timing

- A plugin with **only UI contributions** activates **lazily** — its buttons and
  rows render from `plugin.json` before any code runs, and the code loads on
  first use. This is why the manifest must declare everything.
- A plugin that declares a **service** activates **at boot**, after the window
  is interactive, because a background task has no click to wait for.

## Opening what you declared

Declaring a tab or a modal does not show it. A command handler opens it:

```js
ctx.ui.command('acme.my-plugin.open', () => ctx.ui.openTab('acme.my-plugin.tab'));
```

`openTab` / `openModal` throw when the id was never declared, or when no
renderer has been registered for it yet.

## Icons

An icon is either a built-in name or an image file inside the plugin folder:

```json
{ "type": "builtin", "name": "board" }
{ "type": "image", "path": "icon.svg" }
```

Built-in names: lines, plus, git2, chat, chev, board, git, folder, bot, search,
gear, term, key, dots, bell, phone.

Inline SVG is **not** accepted: the host reserves raw markup for its own
compile-time icons. An image icon does not follow the theme's text colour.

## What a plugin cannot do

- **Add a backend command.** Plugins are frontend-only. They compose the
  allowlisted read-only commands in `backend.invoke`; anything needing a new
  Rust command is out of scope and should be said so plainly rather than faked.
- **Declare keyboard shortcuts.** Commands appear in the palette; binding keys
  is not supported yet.
- **Contribute a theme or palette.**
