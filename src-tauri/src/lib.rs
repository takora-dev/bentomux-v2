// Bentomux Tauri backend library root.
// Modules are ported phase-by-phase from the Electron main process
// (see MIGRATION_TO_TAURI.md). Phase 1 creates the skeleton; each
// module gains its implementation in its dedicated phase.

pub mod state;
pub mod split_tree;
pub mod shell;
pub mod pty;
pub mod detect;
pub mod agents;
pub mod agent_hooks;
pub mod bridge;
pub mod bridge_config;
pub mod commands;
pub mod git;
pub mod overlay;
pub mod remote;
pub mod runtime;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let process_start = std::time::Instant::now();
    // Build AppStateManager before the Tauri Builder so it can be registered
    // via builder.manage() — state is then available before WebView2
    // initialises, making the Windows "state not managed" boot error impossible.
    let app_state = state::AppStateManager::new(state::AppStateManager::pre_build_path());
    let mut builder = tauri::Builder::default();
    builder = builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        .manage(app_state);
    builder = builder.setup(move |app| {
        eprintln!("[perf] backend-ready-ms={}", process_start.elapsed().as_millis());
        use tauri::Emitter;
        use tauri::Manager;
        let handle = app.handle().clone();
        let pty = pty::PtyManager::new(Some(handle.clone()));
        app.manage(pty);
        git::init_watch(handle.clone());
        /* boot-time remote restore: mirrors Electron's index.ts startRemote()
           call when prefs.remote.enabled was persisted true from a prior
           session — otherwise the panel shows "On" but never actually starts. */
        remote::restore_on_startup(&handle, app.state::<state::AppStateManager>().inner());
        /* agent runtime detection: headless screen feed + process poller */
        runtime::init(handle.clone());
        /* approval bridge: unix socket the managed agent hooks write to */
        bridge::start_bridge(handle.clone(), &app.state::<pty::PtyManager>());
        /* track the last-known maximize state on the main window so we only
           emit `win:maximized` on the actual OS transition (mirrors
           Electron's `win.on('maximize'/'unmaximize')` pattern in
           src/main/window.ts). The AtomicBool is shared between the
           setup-time window event handler and win_toggle_maximize. */
        if let Some(main) = app.get_webview_window("main") {
            let last_max = Arc::new(AtomicBool::new(main.is_maximized().unwrap_or(false)));
            app.manage(WindowMaxState(last_max.clone()));
            let app_for_event = handle.clone();
            let last_for_event = last_max.clone();
            let main_for_event = main.clone();
            main.on_window_event(move |ev| match ev {
                tauri::WindowEvent::Resized(_) => {
                    let now = main_for_event.is_maximized().unwrap_or(false);
                    let prev = last_for_event.swap(now, Ordering::SeqCst);
                    if now != prev {
                        let _ = app_for_event.emit("win:maximized", now);
                    }
                }
                _ => {}
            });
        }

        Ok(())
    });
    builder
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::prefs_update,
            commands::workspace_choose,
            commands::workspace_add,
            commands::workspace_remove,
            commands::workspace_reorder,
            commands::workspace_active,
            commands::tab_reorder,
            commands::tab_restore,
            commands::tab_create,
            commands::tab_split,
            commands::tab_rename,
            commands::tab_set_dir,
            commands::tab_close_pane,
            commands::tab_close,
            commands::pty_write,
            commands::pty_resize,
            commands::git_status,
            commands::git_diff,
            commands::git_diff_stat,
            commands::git_push,
            commands::git_remote_info,
            commands::git_branch_for,
            commands::agents_list,
            commands::agents_config,
            commands::agents_set_model_settings,
            commands::agent_hooks_status,
            commands::agent_hooks_install,
            commands::agent_hooks_uninstall,
            commands::agent_approval_resolve,
            commands::agent_approval_pending,
            commands::agent_approval_hide,
            commands::agent_set_active_tab,
            commands::res_list,
            commands::res_save,
            commands::res_delete,
            commands::res_toggle,
            commands::remote_info,
            commands::remote_set_enabled,
            commands::remote_set_port,
            commands::win_minimize,
            commands::win_toggle_maximize,
            commands::win_toggle_fullscreen,
            commands::win_close,
            commands::shutdown_for_update,
            commands::temp_write_file,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            /* ensure pending hook connections are closed on exit so agent
               PermissionRequests don't hang waiting for our directive
               (port of Electron's before-quit → stopBridge in bridge.ts) */
            if let tauri::RunEvent::ExitRequested { .. } = event {
                bridge::stop_bridge();
                crate::remote::stop_tunnel();
                crate::remote::stop_remote();
            }
        });
}

/* shared maximize-state guard; `win_toggle_maximize` reads/swaps it after
   the OS toggle and emits the event so the renderer's `onMaximized` cb
   fires on programmatic toggles too (Electron parity). */
pub struct WindowMaxState(pub Arc<AtomicBool>);
