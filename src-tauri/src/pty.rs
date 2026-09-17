/* ---------------- one pty per terminal tab ----------------
   Rust port of src/main/pty.ts using portable-pty. The master handle is
   kept on the Term so resize works, and a child killer is kept so kill
   works while a waiter thread reaps the real exit code. */

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::Emitter;

use crate::bridge_config::bridge_env_for;
use crate::shell::{resolve_shell, ShellChoice};
use crate::split_tree::{
    first_leaf_id, leaf_ids, leaf_node, remove_leaf, remap_leaves, tree_from_legacy, PaneNode,
};
use crate::state::{TabRec, WorkspaceRec};

pub struct Term {
    pub id: String,
    pub workspace_id: String,
    pub pid: u32,
    alive: Arc<AtomicBool>,
    master: Box<dyn MasterPty + Send>,
    writer: Option<Box<dyn Write + Send>>,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

/* snapshot of a term's identity for callers that don't need the handles */
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TermInfo {
    pub id: String,
    pub workspace_id: String,
    pub pid: u32,
    pub alive: bool,
}

pub struct PtyManager {
    terms: Arc<Mutex<HashMap<String, Term>>>,
    app: Option<tauri::AppHandle>,
    /* observers of raw pty output (agent detection feeds a headless parser) */
    data_tx: tokio::sync::broadcast::Sender<(String, String)>,
    exit_tx: tokio::sync::broadcast::Sender<(String, i32)>,
}

fn decode_pty_bytes(carry: &mut Vec<u8>, bytes: &[u8]) -> Option<String> {
    carry.extend_from_slice(bytes);

    let emit_len = match std::str::from_utf8(carry) {
        Ok(_) => carry.len(),
        Err(err) if err.error_len().is_none() => err.valid_up_to(),
        Err(_) => carry.len(),
    };

    if emit_len == 0 {
        return None;
    }

    let chunk = String::from_utf8_lossy(&carry[..emit_len]).into_owned();
    carry.drain(..emit_len);
    Some(chunk)
}

impl PtyManager {
    pub fn new(app: Option<tauri::AppHandle>) -> Self {
        let (data_tx, _) = tokio::sync::broadcast::channel(1024);
        let (exit_tx, _) = tokio::sync::broadcast::channel(256);
        PtyManager { terms: Arc::new(Mutex::new(HashMap::new())), app, data_tx, exit_tx }
    }

    /* subscribe to raw pty output (id, chunk) */
    pub fn on_term_data(&self) -> tokio::sync::broadcast::Receiver<(String, String)> {
        self.data_tx.subscribe()
    }

    /* subscribe to term exits (id, exit code) */
    pub fn on_term_exit(&self) -> tokio::sync::broadcast::Receiver<(String, i32)> {
        self.exit_tx.subscribe()
    }

    pub fn create_term(
        &self,
        workspace_id: &str,
        workspace_path: &str,
        shell_pref: Option<&str>,
    ) -> Result<String, String> {
        /* resolved per spawn so a settings change applies to the next pane
           without an app restart */
        let shell: ShellChoice = resolve_shell(shell_pref);
        let id = new_term_id();

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| format!("PTY creation failed: {}", e))?;

        let mut cmd = CommandBuilder::new(&shell.file);
        for arg in &shell.args {
            cmd.arg(arg);
        }
        cmd.cwd(workspace_path);
        cmd.env("TERM", "xterm-256color");
        for (k, v) in bridge_env_for(&id) {
            cmd.env(k, v);
        }

        let mut child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("Shell spawn failed: {}", e))?;

        let pid = child.process_id().unwrap_or(0);
        let writer = pair.master.take_writer().map_err(|e| format!("writer failed: {}", e))?;
        let reader = pair.master.try_clone_reader().map_err(|e| format!("reader failed: {}", e))?;
        let killer = child.clone_killer();
        let alive = Arc::new(AtomicBool::new(true));

        let term = Term {
            id: id.clone(),
            workspace_id: workspace_id.to_string(),
            pid,
            alive: alive.clone(),
            master: pair.master,
            writer: Some(writer),
            killer,
        };
        self.terms.lock().unwrap().insert(id.clone(), term);

        /* data reading task (blocking reads need their own thread) */
        let data_tx = self.data_tx.clone();
        let app = self.app.clone();
        let term_id = id.clone();
        std::thread::spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 8192];
            let mut carry = Vec::new();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break, /* EOF */
                    Ok(n) => {
                        let Some(chunk) = decode_pty_bytes(&mut carry, &buf[..n]) else { continue };
                        let _ = data_tx.send((term_id.clone(), chunk.clone()));
                        if let Some(app) = &app {
                            let _ = app.emit("pty:data", (term_id.clone(), chunk));
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        /* waiter thread reaps the child and reports the real exit code */
        let exit_tx = self.exit_tx.clone();
        let app = self.app.clone();
        let term_id = id.clone();
        std::thread::spawn(move || {
            let code = child.wait().ok().map(|s| s.exit_code() as i32).unwrap_or(0);
            alive.store(false, Ordering::SeqCst);
            let _ = exit_tx.send((term_id.clone(), code));
            if let Some(app) = &app {
                let _ = app.emit("pty:exit", (term_id, code));
            }
        });

        Ok(id)
    }

    /* openTabs were persisted with their old ids; fresh shells get new ids,
       so we re-spawn and let the caller re-key the list */
    pub fn restore_terms(&self, workspaces: &[WorkspaceRec], open_tabs: &[TabRec]) -> Vec<TabRec> {
        let mut created = Vec::new();
        for tab in open_tabs {
            let Some(ws) = workspaces.iter().find(|w| w.id == tab.workspace_id) else { continue };
            let tree: PaneNode = tab.split_tree.clone().unwrap_or_else(|| leaf_node(&tab.id));
            let mut map = HashMap::new();
            for old in leaf_ids(&tree) {
                match self.create_term(&ws.id, &ws.path, None) {
                    Ok(fresh) => {
                        map.insert(old, fresh);
                    }
                    Err(e) => eprintln!("[bentomux] pane restore failed for {} {}", ws.path, e),
                }
            }
            if map.is_empty() {
                continue;
            }
            /* prune leaves whose shell could not be spawned */
            let mut fresh: Option<PaneNode> = Some(remap_leaves(tree.clone(), &map));
            for old in leaf_ids(&tree) {
                if map.contains_key(&old) {
                    continue;
                }
                match fresh.and_then(|f| remove_leaf(f, &old)) {
                    Some(f) => fresh = Some(f),
                    None => {
                        fresh = None;
                        break;
                    }
                }
            }
            let Some(fresh) = fresh else { continue };
            created.push(match fresh {
                PaneNode::Leaf { id } => TabRec {
                    id,
                    workspace_id: ws.id.clone(),
                    split_tree: None,
                    title: tab.title.clone(),
                },
                split @ PaneNode::Split { .. } => TabRec {
                    id: first_leaf_id(&split).to_string(),
                    workspace_id: ws.id.clone(),
                    split_tree: Some(split),
                    title: tab.title.clone(),
                },
            });
        }
        created
    }

    pub fn get_term(&self, id: &str) -> Option<TermInfo> {
        self.terms.lock().unwrap().get(id).map(|t| TermInfo {
            id: t.id.clone(),
            workspace_id: t.workspace_id.clone(),
            pid: t.pid,
            alive: t.alive.load(Ordering::SeqCst),
        })
    }

    pub fn write_term(&self, id: &str, data: &str) -> Result<(), String> {
        crate::runtime::note_user_input(id);
        let mut map = self.terms.lock().unwrap();
        let Some(term) = map.get_mut(id) else {
            return Err(format!("Terminal not found: {}", id));
        };
        let Some(writer) = term.writer.as_mut() else {
            return Err(format!("Terminal writer unavailable: {}", id));
        };
        writer.write_all(data.as_bytes()).map_err(|e| format!("PTY write failed: {}", e))?;
        writer.flush().map_err(|e| format!("PTY flush failed: {}", e))
    }

    pub fn resize_term(&self, id: &str, cols: u16, rows: u16) -> Result<(), String> {
        let map = self.terms.lock().unwrap();
        let Some(term) = map.get(id) else {
            return Ok(()); /* pane gone: nothing to resize, not an error */
        };
        if !term.alive.load(Ordering::SeqCst) {
            return Ok(());
        }
        term.master
            .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| format!("Resize failed: {}", e))?;
        /* keep the headless screen model (remote mirror) in step with the
           real terminal so its column-position parsing doesn't drift */
        crate::detect::screen::resize(id, cols, rows);
        Ok(())
    }

    pub fn kill_term(&self, id: &str) -> bool {
        let removed = {
            let mut map = self.terms.lock().unwrap();
            map.remove(id)
        };
        match removed {
            Some(mut term) => {
                if term.alive.load(Ordering::SeqCst) {
                    let _ = term.killer.kill(); /* already gone is fine */
                }
                true
            }
            None => false,
        }
    }

    pub fn kill_terms_for_workspace(&self, workspace_id: &str) {
        let ids: Vec<String> = {
            let map = self.terms.lock().unwrap();
            map.values()
                .filter(|t| t.workspace_id == workspace_id)
                .map(|t| t.id.clone())
                .collect()
        };
        for id in ids {
            self.kill_term(&id);
        }
    }

    pub fn kill_all_terms(&self) {
        let ids: Vec<String> = {
            let map = self.terms.lock().unwrap();
            map.keys().cloned().collect()
        };
        for id in ids {
            self.kill_term(&id);
        }
    }

    pub fn live_terms(&self) -> Vec<TermInfo> {
        self.terms
            .lock()
            .unwrap()
            .values()
            .filter(|t| t.alive.load(Ordering::SeqCst))
            .map(|t| TermInfo {
                id: t.id.clone(),
                workspace_id: t.workspace_id.clone(),
                pid: t.pid,
                alive: true,
            })
            .collect()
    }
}

/* 't-' + millis in base36 + 4 random base36 chars, like the TS version */
pub fn new_term_id() -> String {
    use rand::Rng;
    let millis = (chrono::Utc::now().timestamp_millis().max(0)) as u64;
    let rnd: u32 = rand::thread_rng().gen_range(0..36u32.pow(4));
    format!(
        "t-{}{:0>4}",
        crate::split_tree::to_base36(millis),
        crate::split_tree::to_base36(rnd as u64)
    )
}

/* legacy tabs whose tree is absent still resolve through treeFromLegacy in
   state.rs; this helper is kept for callers that need the same shape */
pub fn legacy_tree(ids: &[String], stacked: bool) -> Option<PaneNode> {
    tree_from_legacy(
        ids,
        Some(if stacked { crate::split_tree::Dir::H } else { crate::split_tree::Dir::V }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> String {
        let dir = std::env::temp_dir().join(format!(
            "bentomux-pty-{}-{}-{}",
            tag,
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.to_string_lossy().into_owned()
    }

    fn ws(id: &str) -> WorkspaceRec {
        WorkspaceRec { id: id.into(), path: temp_dir(id), name: id.into() }
    }

    #[test]
    fn test_decode_pty_bytes_preserves_split_utf8() {
        let mut carry = Vec::new();
        assert_eq!(decode_pty_bytes(&mut carry, &[0xE2, 0x94]), None);
        assert_eq!(decode_pty_bytes(&mut carry, &[0x80]), Some("─".to_string()));
    }

    #[tokio::test]
    async fn test_spawn_write_read_exit() {
        let mgr = PtyManager::new(None);
        let mut data_rx = mgr.on_term_data();
        let mut exit_rx = mgr.on_term_exit();

        let workspace = ws("spawn");
        /* windows: cmd starts instantly and skips user pwsh profiles; unix:
           the detected default (bash) as before */
        let shell_pref = if cfg!(windows) { Some("cmd") } else { None };
        let id = mgr.create_term(&workspace.id, &workspace.path, shell_pref).expect("spawn");

        let info = mgr.get_term(&id).expect("term registered");
        assert!(info.alive);
        assert!(info.pid > 0);

        /* echo a marker and expect it back through the pty. \r submits the
           line the way a real Enter keystroke does (\n alone does not in
           cmd/PowerShell); bash tolerates the trailing \r.

           Reading accumulates every chunk: ConPTY renders the screen and
           splits one echoed line across several reads, so a per-chunk check
           misses a marker that did arrive. Silence is not failure either --
           the first cmd/PowerShell prompt can take a while on a cold CI
           runner, and ConPTY drops input written before the client attaches,
           so a quiet gap re-sends the command instead of giving up. */
        let command = "echo BENTOMUX_TEST_MARKER\r\n";
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut seen = String::new();
        let mut last_write: Option<std::time::Instant> = None;
        while !seen.contains("BENTOMUX_TEST_MARKER") {
            if std::time::Instant::now() >= deadline {
                panic!("marker never appeared in pty output; saw {seen:?}");
            }
            match tokio::time::timeout(std::time::Duration::from_millis(200), data_rx.recv()).await {
                Ok(Ok((tid, chunk))) => {
                    assert_eq!(tid, id);
                    seen.push_str(&chunk);
                }
                /* this test's own receiver can only lag by ignoring the stream */
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
                /* channel closed: nothing more will ever arrive */
                Ok(Err(_)) => break,
                Err(_) => {
                    let due = last_write
                        .map(|at: std::time::Instant| at.elapsed() >= std::time::Duration::from_secs(2))
                        .unwrap_or(true);
                    if due {
                        mgr.write_term(&id, command).expect("write");
                        last_write = Some(std::time::Instant::now());
                    }
                }
            }
        }
        assert!(seen.contains("BENTOMUX_TEST_MARKER"));

        mgr.kill_term(&id);
        let exited = tokio::time::timeout(std::time::Duration::from_secs(5), exit_rx.recv()).await;
        assert!(exited.is_ok(), "no exit event after kill");
        assert_eq!(mgr.get_term(&id), None, "term removed from map on kill");
    }

    #[tokio::test]
    async fn test_resize_and_unknown_term_errors() {
        let mgr = PtyManager::new(None);
        let workspace = ws("resize");
        let id = mgr.create_term(&workspace.id, &workspace.path, None).expect("spawn");

        mgr.resize_term(&id, 100, 30).expect("resize");
        /* unknown terms resize as a no-op, write as an error (like TS) */
        mgr.resize_term("t-nope", 80, 24).unwrap();
        assert!(mgr.write_term("t-nope", "hi").is_err());
        assert!(!mgr.kill_term("t-nope"));
        mgr.kill_term(&id);
    }

    #[test]
    fn test_restore_terms_remaps_and_prunes() {
        let mgr = PtyManager::new(None);
        let workspace = ws("restore");

        /* a persisted tab with a two-pane split tree under old ids */
        let old_tree = crate::split_tree::split_leaf(
            leaf_node("old-a"),
            "old-a",
            crate::split_tree::Dir::V,
            "old-b",
            "k1",
        );
        let open_tabs = vec![TabRec {
            id: "old-a".into(),
            workspace_id: workspace.id.clone(),
            split_tree: Some(old_tree),
            title: Some("restored".into()),
        }];

        let restored = mgr.restore_terms(&[workspace.clone()], &open_tabs);
        assert_eq!(restored.len(), 1);
        let tab = &restored[0];
        assert_eq!(tab.title.as_deref(), Some("restored"));
        let tree = tab.split_tree.as_ref().expect("split preserved");
        let leaves = leaf_ids(tree);
        assert_eq!(leaves.len(), 2);
        assert!(leaves.iter().all(|l| mgr.get_term(l).is_some()), "fresh panes registered");
        assert!(!leaves.contains(&"old-a".to_string()));
        assert_eq!(tab.id, first_leaf_id(tree));

        /* single-pane legacy tab (no tree) restores as a plain tab */
        let legacy: Vec<String> = vec!["x1".into()];
        let single = TabRec {
            id: "x1".into(),
            workspace_id: workspace.id.clone(),
            split_tree: tree_from_legacy(&legacy, None),
            title: None,
        };
        let restored = mgr.restore_terms(&[workspace.clone()], &[single]);
        assert_eq!(restored.len(), 1);
        assert!(restored[0].split_tree.is_none());
        assert!(mgr.get_term(&restored[0].id).is_some());

        /* unknown workspace tabs are skipped */
        let orphan = TabRec {
            id: "z1".into(),
            workspace_id: "ws-gone".into(),
            split_tree: None,
            title: None,
        };
        assert!(mgr.restore_terms(&[workspace], &[orphan]).is_empty());

        mgr.kill_all_terms();
        assert!(mgr.live_terms().is_empty());
    }

    #[test]
    fn test_kill_terms_for_workspace() {
        let mgr = PtyManager::new(None);
        let a = ws("ws-a");
        let b = ws("ws-b");
        let ta = mgr.create_term(&a.id, &a.path, None).unwrap();
        let tb = mgr.create_term(&b.id, &b.path, None).unwrap();

        mgr.kill_terms_for_workspace(&a.id);
        assert_eq!(mgr.get_term(&ta), None);
        assert!(mgr.get_term(&tb).is_some());

        mgr.kill_all_terms();
        assert_eq!(mgr.get_term(&tb), None);
    }

    #[test]
    fn test_term_id_shape_and_uniqueness() {
        let ids: Vec<String> = (0..100).map(|_| new_term_id()).collect();
        assert!(ids.iter().all(|id| id.starts_with("t-")));
        let uniq: std::collections::HashSet<&String> = ids.iter().collect();
        assert_eq!(uniq.len(), 100);
    }
}
