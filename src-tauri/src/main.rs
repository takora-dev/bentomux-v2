// Prevents an additional console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    /* the same binary doubles as the persistent pty host: the app spawns
       `bentomux --pty-host` so the panes outlive the UI process */
    if std::env::args().any(|arg| arg == bentomux_lib::pty_host::HOST_FLAG) {
        std::process::exit(bentomux_lib::pty_host::run_host());
    }
    bentomux_lib::run()
}
