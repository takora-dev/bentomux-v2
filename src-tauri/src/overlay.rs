/* ---------------- approval overlay window (floating "dynamic island") ----------------
Port of src/main/overlay.ts from the Electron build. When Bentomux is
not the frontmost window, a PermissionRequest pops a small always-on-top
pill at the top-center of the SCREEN so the decision can be made without
switching to Bentomux (e.g. while browsing).

The overlay loads the bundled `approval.html` page (built by Vite),
which subscribes to the same `agent:approval` / `agent:approvalClosed`
events and resolves/denies/jumps over the shared IPC bridge
(resolveApproval, approvalJump). The window never takes focus — the
user's foreground app keeps keyboard input. Resizing persists the size
to prefs.approvalOverlay. */

use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};

const OVERLAY_MIN_W: f64 = 480.0;
const OVERLAY_MIN_H: f64 = 220.0;
const LABEL: &str = "approval-overlay";

/* the Vite dev server / bundled assets live at the same origin as the main
window, so approval.html resolves naturally in both dev and prod. */
const OVERLAY_PAGE: &str = "approval.html";

fn current_position(app: &AppHandle, w: f64, h: f64) -> (f64, f64) {
    /* center horizontally over the main window's monitor work area, anchored
    10px from the top of the screen — mirrors Electron's
    screen.getPrimaryDisplay().workArea math. */
    let win = app.get_webview_window("main");
    let (wx, wy, ww, wh) = match win {
        Some(w) if w.current_monitor().ok().flatten().is_some() => {
            let m = w.current_monitor().unwrap().unwrap();
            let size = m.size();
            let pos = m.position();
            let scale = w.scale_factor().unwrap_or(1.0);
            (
                pos.x as f64,
                pos.y as f64,
                size.width as f64 / scale as f64,
                size.height as f64 / scale as f64,
            )
        }
        _ => (0.0, 0.0, 1920.0, 1080.0),
    };
    let x = wx + (ww - w) / 2.0;
    let y = wy + (10.0f64).min(wh - h);
    (x.max(0.0), y.max(0.0))
}

fn pref_size(app: &AppHandle) -> (f64, f64) {
    let pref = app
        .state::<crate::state::AppStateManager>()
        .get_state()
        .prefs
        .approval_overlay;
    match pref {
        Some(p) if p.w >= OVERLAY_MIN_W && p.h >= OVERLAY_MIN_H => (p.w, p.h),
        _ => (OVERLAY_MIN_W, OVERLAY_MIN_H),
    }
}

/* create the overlay window on first request; subsequent requests just
re-show it and re-center. The page itself subscribes to `agent:approval`
(which the bridge already emits for every request) and renders it, so no
per-request payload is passed through the window URL. */
pub fn show_approval_overlay(app: &AppHandle) {
    let (w, h) = pref_size(app);
    let (x, y) = current_position(app, w, h);

    if let Some(existing) = app.get_webview_window(LABEL) {
        let _ = existing.set_position(LogicalPosition::new(x, y));
        let _ = existing.set_size(LogicalSize::new(w, h));
        /* show WITHOUT focusing — the overlay must never steal keyboard from
        the user's foreground app (Electron's showInactive + focusable:false) */
        let _ = existing.show();
        return;
    }

    let app_resize = app.clone();
    let builder = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App(OVERLAY_PAGE.into()))
        .title("Bentomux — approval")
        .inner_size(w, h)
        .position(x, y)
        .min_inner_size(OVERLAY_MIN_W, OVERLAY_MIN_H)
        .decorations(false)
        .resizable(true)
        .skip_taskbar(true)
        /* never steals focus from the user's foreground app */
        .focused(false)
        .always_on_top(true)
        .visible(false);

    match builder.build() {
        Ok(window) => {
            /* persist resize (debounced by OS resize cadence) + keep centered */
            let window_resize = window.clone();
            window.on_window_event(move |ev| {
                if let WindowEvent::Resized(_) = ev {
                    if let Ok(size) = window_resize.inner_size() {
                        let scale = window_resize.scale_factor().unwrap_or(1.0);
                        let w = size.width as f64 / scale;
                        let h = size.height as f64 / scale;
                        let state = app_resize.state::<crate::state::AppStateManager>();
                        state.patch_state(|s| {
                            s.prefs.approval_overlay = Some(crate::state::OverlaySize {
                                w: w.round(),
                                h: h.round(),
                            });
                        });
                        let (x, y) = current_position(&app_resize, w, h);
                        let _ = window_resize.set_position(LogicalPosition::new(x, y));
                    }
                }
            });
            let _ = window.show();
        }
        Err(e) => {
            /* do not crash the app if the window can't be created; the
            request is still visible in the main window / bridge. */
            eprintln!("[bentomux] approval overlay failed to open: {e}");
        }
    }
}
