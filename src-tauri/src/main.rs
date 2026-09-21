// Prevents an additional console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    /* the same binary doubles as the persistent pty host: the app spawns
       `bentomux --pty-host` so the panes outlive the UI process */
    if std::env::args().any(|arg| arg == bentomux_lib::pty_host::HOST_FLAG) {
        std::process::exit(bentomux_lib::pty_host::run_host());
    }

    /* authoring surfaces: validate a plugin folder, or start without
       third-party plugins. Both exit before any window is created. */
    if args.iter().any(|a| a == bentomux_lib::plugin::cli::VALIDATE_FLAG) {
        std::process::exit(bentomux_lib::plugin::cli::run_validate(&args));
    }

    bentomux_lib::run()
}
