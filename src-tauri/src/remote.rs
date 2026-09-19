/* ---------------- remote control (phone browser) ----------------
   Rust port of src/main/remote.ts. HTTPS-only: the local server binds
   loopback and is reachable solely through the bundled cloudflared
   quick tunnel, whose public https://trycloudflare.com URL (token-
   embedded) is the pairing QR shown in Settings. Every route (page and
   WS) requires the pairing token from prefs.remote.token — the token
   now grants FULL CONTROL (watch, switch panes, type into the watched
   pane, approve/deny), so treat a leaked URL as shell access. Screen
   text comes from the headless render in detect/screen.rs (plain text,
   escape sequences consumed); watched panes are re-serialized on a
   fixed tick while their output is moving. Remote decisions go through
   the same resolve_approval() the local overlay uses, so every surface
   closes via agent:approvalClosed. */

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State as AxState;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use base64::Engine;
use futures_util::stream::SplitSink;
use futures_util::SinkExt;
use serde::Serialize;
use serde_json::json;
use tauri::{path::BaseDirectory, Manager};

use crate::state::{AppStateManager, RemotePrefs};

pub const DEFAULT_REMOTE_PORT: u16 = 8765;

/* watched panes re-serialize at most this often, and only while output
   is actually moving */
const WATCH_TICK_MS: u64 = 250;

/* The pairing token in the QR is exchanged once for an HttpOnly cookie and
   never used as a session: it stays out of browser history, Referer headers,
   and any proxy log that records query strings. */
const SESSION_COOKIE: &str = "bentomux_session";
const SESSION_TTL: Duration = Duration::from_secs(12 * 60 * 60);
const MAX_SESSIONS: usize = 32;

/* bounds on what a single socket may send, and how fast */
const MAX_WS_MESSAGE: usize = 64 * 1024;
const MAX_WRITE_BYTES: usize = 8 * 1024;
const MAX_INBOUND_PER_SEC: u32 = 200;
/* pairing-token guesses allowed per minute across all connections */
const MAX_AUTH_PER_MIN: u32 = 30;

/* ---------------- renderer-facing types (shared/types) ---------------- */

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RemotePaneInfo {
    pub id: String,
    pub title: String,
    pub workspace: String,
    pub state: Option<crate::runtime::AgentRunState>,
    pub runtime: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RemotePairing {
    pub enabled: bool,
    pub running: bool,
    pub port: u16,
    pub token: String,
    pub urls: Vec<String>,
    pub qr: Option<String>,
    pub error: Option<String>,
    pub tunnel_url: Option<String>,
    pub tunnel_qr: Option<String>,
    pub tunnel_error: Option<String>,
}

/* ---------------- outbound message to phone clients ---------------- */

#[derive(Clone)]
enum RemoteMsg {
    /* already-serialized JSON: hello / panes / status / approval / approvalClosed */
    Json(String),
    View { pane_id: String, text: String, html: String },
    Gone { pane_id: String },
}

fn js(v: &impl Serialize) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string())
}

/* live server handle; None while stopped. LAST_ERROR remembers the most
   recent bind failure so the Settings surface can show it. */
struct ServerState {
    kill_tx: tokio::sync::watch::Sender<bool>,
    running: Arc<AtomicBool>,
}

static SERVER: std::sync::OnceLock<Mutex<Option<ServerState>>> = std::sync::OnceLock::new();
fn server() -> &'static Mutex<Option<ServerState>> {
    SERVER.get_or_init(|| Mutex::new(None))
}

static LAST_ERROR: std::sync::OnceLock<Mutex<Option<String>>> = std::sync::OnceLock::new();
fn last_error() -> &'static Mutex<Option<String>> {
    LAST_ERROR.get_or_init(|| Mutex::new(None))
}

/* feed callbacks registered once reuse the CURRENT_OUT channel each start,
   so a restart never double-registers or writes into a dead channel. */
static CURRENT_OUT: std::sync::OnceLock<Mutex<Option<tokio::sync::broadcast::Sender<RemoteMsg>>>> =
    std::sync::OnceLock::new();
fn current_out() -> &'static Mutex<Option<tokio::sync::broadcast::Sender<RemoteMsg>>> {
    CURRENT_OUT.get_or_init(|| Mutex::new(None))
}

static FEEDS_ONCE: std::sync::Once = std::sync::Once::new();

pub fn remote_running() -> bool {
    server().lock().unwrap().is_some()
}

fn remote_port(prefs: &crate::state::Prefs) -> u16 {
    prefs.remote.as_ref().and_then(|r| r.port).unwrap_or(DEFAULT_REMOTE_PORT)
}

fn remote_enabled(prefs: &crate::state::Prefs) -> bool {
    prefs.remote.as_ref().and_then(|r| r.enabled).unwrap_or(false)
}

/* generated on first use so paired devices survive restarts */
fn ensure_token(state: &AppStateManager) -> String {
    let now = state.get_state().prefs;
    if let Some(tok) = now.remote.as_ref().and_then(|r| r.token.clone()) {
        return tok;
    }
    let token = random_token();
    let tok = token.clone();
    state.patch_prefs(|p| {
        let r = p.remote.get_or_insert_with(|| RemotePrefs {
            enabled: None,
            port: None,
            token: None,
        });
        r.token = Some(tok);
    });
    token
}

fn random_token() -> String {
    let bytes: [u8; 24] = rand::random();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/* ---------------- pane + status snapshots ---------------- */

fn pane_list(app: &tauri::AppHandle) -> Vec<RemotePaneInfo> {
    use crate::split_tree::{leaf_node, tree_has_leaf};
    let state = app.state::<AppStateManager>().get_state();
    let statuses = crate::runtime::latest_runtime_statuses();
    let mut out = Vec::new();
    for t in app.state::<crate::pty::PtyManager>().live_terms() {
        let rec = state.open_tabs.iter().find(|r| {
            let tree = r.split_tree.clone().unwrap_or_else(|| leaf_node(&r.id));
            tree_has_leaf(&tree, &t.id)
        });
        let Some(rec) = rec else { continue };
        let ws = state.workspaces.iter().find(|w| w.id == rec.workspace_id);
        let status = statuses.get(&t.id);
        out.push(RemotePaneInfo {
            id: t.id,
            title: rec
                .title
                .clone()
                .filter(|s| !s.trim().is_empty())
                .or_else(|| ws.map(|w| w.name.clone()))
                .unwrap_or_else(|| "Terminal".to_string()),
            workspace: ws.map(|w| w.name.clone()).unwrap_or_default(),
            state: status.and_then(|s| s.state.clone()),
            runtime: status.and_then(|s| s.runtime.clone()),
        });
    }
    out
}

/* ---------------- remote page asset ---------------- */

const PAGE_FALLBACK: &str = "<!doctype html><meta charset=utf-8><title>Bentomux remote</title>\
<h1>Bentomux remote</h1><p>Pairing page not found (remote-page.html resource missing).</p>";

fn remote_page_html(app: &tauri::AppHandle) -> String {
    app.path()
        .resolve("../resources/remote-page.html", BaseDirectory::Resource)
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_else(|| PAGE_FALLBACK.to_string())
}

/* ---------------- http + ws handlers (axum) ---------------- */

#[derive(Clone)]
struct WsCtx {
    token: Arc<String>,
    sessions: Arc<Sessions>,
    out: tokio::sync::broadcast::Sender<RemoteMsg>,
    app: tauri::AppHandle,
}

/* ---------------- auth: cookie sessions + rate limits ---------------- */

/* byte-wise compare; the token length is fixed and public, the bytes are what
   must not be guessable one byte at a time */
fn ct_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/* fixed-window counter: no dependency, and enough to stop a leaked URL from
   hammering the socket or grinding the pairing token */
struct RateLimit {
    start: Instant,
    count: u32,
    max: u32,
    window: Duration,
}

impl RateLimit {
    fn new(max: u32, window: Duration) -> Self {
        Self { start: Instant::now(), count: 0, max, window }
    }

    fn allow(&mut self) -> bool {
        let now = Instant::now();
        if now.duration_since(self.start) >= self.window {
            self.start = now;
            self.count = 0;
        }
        self.count += 1;
        self.count <= self.max
    }
}

static AUTH_LIMIT: std::sync::OnceLock<Mutex<RateLimit>> = std::sync::OnceLock::new();

fn auth_allowed() -> bool {
    AUTH_LIMIT
        .get_or_init(|| Mutex::new(RateLimit::new(MAX_AUTH_PER_MIN, Duration::from_secs(60))))
        .lock()
        .unwrap()
        .allow()
}

/* live browser sessions. Capped and expired so a long-running app cannot
   accumulate them; every use slides the expiry forward. */
#[derive(Default)]
struct Sessions {
    map: Mutex<HashMap<String, Instant>>,
}

impl Sessions {
    fn create(&self) -> String {
        let mut map = self.map.lock().unwrap();
        let now = Instant::now();
        map.retain(|_, seen| now.duration_since(*seen) < SESSION_TTL);
        while map.len() >= MAX_SESSIONS {
            let Some(oldest) = map.iter().min_by_key(|(_, seen)| **seen).map(|(id, _)| id.clone()) else { break };
            map.remove(&oldest);
        }
        let id = random_token();
        map.insert(id.clone(), now);
        id
    }

    fn valid(&self, id: &str) -> bool {
        let mut map = self.map.lock().unwrap();
        let now = Instant::now();
        map.retain(|_, seen| now.duration_since(*seen) < SESSION_TTL);
        let Some(seen) = map.get_mut(id) else { return false };
        *seen = now;
        true
    }
}

fn session_from_cookie(headers: &axum::http::HeaderMap) -> Option<String> {
    let raw = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        if let Some((key, value)) = part.split_once('=') {
            if key.trim() == SESSION_COOKIE {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

fn session_cookie(id: &str, secure: bool) -> String {
    let mut c = format!(
        "{SESSION_COOKIE}={id}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
        SESSION_TTL.as_secs()
    );
    if secure {
        c.push_str("; Secure");
    }
    c
}

/* one response for every auth failure: no oracle telling a guesser whether
   the token existed, only whether it was right */
fn auth_failed() -> axum::response::Response {
    (
        axum::http::StatusCode::UNAUTHORIZED,
        "Bentomux remote: open the pairing URL shown in Settings \u{2192} Remote.",
    )
        .into_response()
}

#[cfg(test)]
mod ws_auth_regression {
    use super::*;

    #[test]
    fn ct_eq_rejects_missing_and_wrong_tokens() {
        assert!(!ct_eq("", "secret-tok"));
        assert!(!ct_eq("wrong", "secret-tok"));
        assert!(!ct_eq("secret-tol", "secret-tok"));
    }

    #[test]
    fn ct_eq_accepts_exact_token() {
        assert!(ct_eq("secret-tok", "secret-tok"));
    }

    #[test]
    fn cookie_round_trips_and_ignores_other_cookies() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            "theme=dark; bentomux_session=sid-1; other=1".parse().unwrap(),
        );
        assert_eq!(session_from_cookie(&headers).as_deref(), Some("sid-1"));
        assert_eq!(session_from_cookie(&axum::http::HeaderMap::new()), None);
    }

    #[test]
    fn session_expires_and_is_required() {
        let sessions = Sessions::default();
        assert!(!sessions.valid("nope"));
        let id = sessions.create();
        assert!(sessions.valid(&id));
        sessions
            .map
            .lock()
            .unwrap()
            .insert(id.clone(), Instant::now() - SESSION_TTL - Duration::from_secs(1));
        assert!(!sessions.valid(&id));
    }

    #[test]
    fn sessions_are_capped() {
        let sessions = Sessions::default();
        for _ in 0..MAX_SESSIONS + 5 {
            sessions.create();
        }
        assert_eq!(sessions.map.lock().unwrap().len(), MAX_SESSIONS);
    }

    #[test]
    fn rate_limit_closes_the_window() {
        let mut rl = RateLimit::new(2, Duration::from_secs(60));
        assert!(rl.allow());
        assert!(rl.allow());
        assert!(!rl.allow());
    }
}

/* The pairing URL carries the token exactly once. It is traded here for an
   HttpOnly session cookie and the browser is redirected to the bare path, so
   the token leaves the address bar before the page even renders. */
async fn handle_page(
    AxState(st): AxState<WsCtx>,
    headers: axum::http::HeaderMap,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> impl IntoResponse {
    if let Some(token) = params.get("t") {
        if !auth_allowed() || !ct_eq(token, st.token.as_str()) {
            return auth_failed();
        }
        /* cloudflared terminates TLS and says so; a direct loopback hit is
           plain HTTP, where a Secure cookie would simply be dropped */
        let secure = headers.get("x-forwarded-proto").and_then(|v| v.to_str().ok()) == Some("https");
        let mut res = axum::http::HeaderMap::new();
        res.insert(axum::http::header::LOCATION, "/".parse().unwrap());
        res.insert(
            axum::http::header::SET_COOKIE,
            session_cookie(&st.sessions.create(), secure).parse().unwrap(),
        );
        return (axum::http::StatusCode::SEE_OTHER, res).into_response();
    }
    if !st.sessions.valid(&session_from_cookie(&headers).unwrap_or_default()) {
        return auth_failed();
    }
    (
        axum::http::StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        remote_page_html(&st.app),
    )
        .into_response()
}

/* the pairing token is never accepted here: a socket has to present the
   session cookie the page exchange issued */
async fn handle_ws(
    ws: WebSocketUpgrade,
    AxState(st): AxState<WsCtx>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    if !st.sessions.valid(&session_from_cookie(&headers).unwrap_or_default()) {
        return auth_failed();
    }
    ws.max_message_size(MAX_WS_MESSAGE)
        .max_frame_size(MAX_WS_MESSAGE)
        .on_upgrade(move |socket| client_loop(socket, st))
        .into_response()
}

async fn send_json(sender: &mut SplitSink<WebSocket, Message>, body: String) -> bool {
    sender.send(Message::Text(body)).await.is_ok()
}

async fn client_loop(ws: WebSocket, st: WsCtx) {
    use futures_util::StreamExt;
    let (mut sender, mut receiver) = ws.split();
    let pane_id: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let mut out_rx = st.out.subscribe();
    /* a phone types in bursts, not floods: anything past this is dropped with
       the connection rather than forwarded into a shell */
    let mut inbound_limit = RateLimit::new(MAX_INBOUND_PER_SEC, Duration::from_secs(1));

    /* everything a fresh client needs: identity, current state */
    send_json(&mut sender, js(&json!({"t": "hello"}))).await;
    send_json(&mut sender, js(&json!({"t": "panes", "panes": pane_list(&st.app)}))).await;
    send_json(&mut sender, js(&json!({"t": "status", "statuses": crate::runtime::latest_runtime_statuses()}))).await;
    for req in crate::bridge::pending_approvals() {
        send_json(&mut sender, js(&json!({"t": "approval", "req": req}))).await;
    }

    loop {
        tokio::select! {
            inbound = receiver.next() => {
                let Some(incoming) = inbound else { break };
                let msg = match incoming { Ok(m) => m, Err(_) => break };
                match msg {
                    Message::Text(text) => {
                        if !inbound_limit.allow() {
                            break;
                        }
                        handle_incoming(text, &pane_id, &mut sender, &st.app).await
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            out = out_rx.recv() => {
                match out {
                    Ok(RemoteMsg::View { pane_id: p, text, html }) => {
                        if pane_id.lock().unwrap().as_deref() == Some(p.as_str()) {
                            if !send_json(&mut sender, js(&json!({"t": "view", "paneId": p, "text": text, "html": html}))).await { break; }
                        }
                    }
                    Ok(RemoteMsg::Gone { pane_id: p }) => {
                        if pane_id.lock().unwrap().as_deref() == Some(p.as_str()) {
                            *pane_id.lock().unwrap() = None;
                            if !send_json(&mut sender, js(&json!({"t": "gone", "paneId": p}))).await { break; }
                        }
                    }
                    Ok(RemoteMsg::Json(body)) => {
                        if !send_json(&mut sender, body).await { break; }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

/* ids of panes that currently exist and are alive; a remote client may only
   watch or type into one of these */
fn live_panes(app: &tauri::AppHandle) -> Vec<String> {
    app.state::<crate::pty::PtyManager>()
        .live_terms()
        .into_iter()
        .map(|t| t.id)
        .collect()
}

/* strict wire messages from the phone; anything malformed is ignored.
   "write" is full control — it types into the pane this connection is
   currently watching. */
async fn handle_incoming(
    text: String,
    pane_id: &Arc<Mutex<Option<String>>>,
    sender: &mut SplitSink<WebSocket, Message>,
    app: &tauri::AppHandle,
) {
    let parsed: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return,
    };
    let Some(m) = parsed.as_object() else { return };
    match m.get("t").and_then(|v| v.as_str()) {
        Some("watch") => {
            if let Some(pid) = m.get("paneId").and_then(|v| v.as_str()) {
                if !live_panes(app).iter().any(|id| id == pid) {
                    return;
                }
                *pane_id.lock().unwrap() = Some(pid.to_string());
                let html = crate::detect::screen::screen_dump_html(pid);
                let text = crate::detect::screen::screen_dump(pid);
                let body = js(&json!({"t": "view", "paneId": pid, "text": text, "html": html}));
                let _ = sender.send(Message::Text(body)).await;
            }
        }
        Some("unwatch") => {
            *pane_id.lock().unwrap() = None;
        }
        Some("write") => {
            let pid = m.get("paneId").and_then(|v| v.as_str());
            let data = m.get("data").and_then(|v| v.as_str());
            let watched = pane_id.lock().unwrap().clone();
            let live = live_panes(app);
            if let Some((pid, data)) = valid_write(watched.as_deref(), pid, data, &live) {
                let _ = app.state::<crate::pty::PtyManager>().write_term(pid, data);
            }
        }
        Some("approve") => {
            let rid = m.get("requestId").and_then(|v| v.as_str()).map(String::from);
            let decision = m.get("decision").and_then(|v| v.as_str());
            if let (Some(rid), Some(dec @ ("allow" | "deny"))) = (rid, decision) {
                crate::bridge::resolve_approval(&rid, dec == "allow");
            }
        }
        _ => {}
    }
}

/* a write is only delivered when it targets the pane this connection is
   currently watching, that pane is still live, and the payload is bounded */
fn valid_write<'a>(
    watched: Option<&'a str>,
    pane_id: Option<&'a str>,
    data: Option<&'a str>,
    live: &[String],
) -> Option<(&'a str, &'a str)> {
    let watched = watched?;
    let pid = pane_id?;
    let data = data?;
    if pid != watched || data.is_empty() || data.len() > MAX_WRITE_BYTES {
        return None;
    }
    if !live.iter().any(|id| id == pid) {
        return None;
    }
    Some((pid, data))
}

#[cfg(test)]
mod write_validation {
    use super::*;

    fn live() -> Vec<String> {
        vec!["t-1".to_string(), "t-2".to_string()]
    }

    #[test]
    fn rejects_missing_or_empty_write() {
        let l = live();
        assert!(valid_write(Some("t-1"), None, Some("ls"), &l).is_none());
        assert!(valid_write(Some("t-1"), Some("t-1"), None, &l).is_none());
        assert!(valid_write(Some("t-1"), Some("t-1"), Some(""), &l).is_none());
    }

    #[test]
    fn rejects_a_pane_this_connection_is_not_watching() {
        let l = live();
        assert!(valid_write(None, Some("t-1"), Some("ls"), &l).is_none());
        assert!(valid_write(Some("t-1"), Some("t-2"), Some("ls"), &l).is_none());
    }

    #[test]
    fn rejects_a_pane_that_is_not_live() {
        assert!(valid_write(Some("t-9"), Some("t-9"), Some("ls"), &live()).is_none());
    }

    #[test]
    fn rejects_an_oversized_payload() {
        let big = "a".repeat(MAX_WRITE_BYTES + 1);
        assert!(valid_write(Some("t-1"), Some("t-1"), Some(&big), &live()).is_none());
    }

    #[test]
    fn accepts_the_watched_live_pane() {
        let (pid, data) = valid_write(Some("t-1"), Some("t-1"), Some("ls -la\r"), &live()).expect("valid");
        assert_eq!(pid, "t-1");
        assert_eq!(data, "ls -la\r");
    }
}

/* ---------------- feeds (pty ticks, approvals, runtime status) ---------------- */

fn broadcast(msg: RemoteMsg) {
    if let Some(tx) = current_out().lock().unwrap().as_ref() {
        let _ = tx.send(msg);
    }
}

fn broadcast_view(pane_id: &str, text: &str) {
    let html = crate::detect::screen::screen_dump_html(pane_id);
    broadcast(RemoteMsg::View { pane_id: pane_id.to_string(), text: text.to_string(), html });
}

/* serialize each watched, recently-active pane once per tick */
fn push_dirty(dirty: &Arc<Mutex<Vec<String>>>) {
    let pending: Vec<String> = dirty.lock().unwrap().drain(..).collect();
    for pane_id in pending {
        broadcast_view(&pane_id, &crate::detect::screen::screen_dump(&pane_id));
    }
}

/* bridge + runtime deliver via push callbacks; wire them once. Each reads
   CURRENT_OUT so a restart reuses the live channel without re-subscribing. */
fn ensure_feeds() {
    FEEDS_ONCE.call_once(|| {
        crate::bridge::on_approval_created(move |req| {
            broadcast(RemoteMsg::Json(js(&json!({"t": "approval", "req": req}))));
        });
        crate::bridge::on_approval_closed(move |request_id| {
            broadcast(RemoteMsg::Json(js(&json!({"t": "approvalClosed", "requestId": request_id}))));
        });
        crate::runtime::on_runtime_update(move |_| {
            broadcast(RemoteMsg::Json(js(&json!({"t": "status", "statuses": crate::runtime::latest_runtime_statuses()}))));
        });
    });
}

/* ---------------- start / stop ---------------- */

pub fn start_remote(app: &tauri::AppHandle, state: &AppStateManager) {
    if remote_running() {
        return;
    }
    ensure_feeds();
    let token = ensure_token(state);
    let port = remote_port(&state.get_state().prefs);

    /* bind synchronously so EADDRINUSE surfaces immediately (like the TS
       listen callback that records lastError before resolving). Loopback
       only: the phone path is the cloudflared tunnel, so plain-HTTP LAN
       access is unreachable by construction. */
    let listener = match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            *last_error().lock().unwrap() = Some(format!("Port {port} is already in use"));
            return;
        }
        Err(e) => {
            *last_error().lock().unwrap() = Some(e.to_string());
            return;
        }
    };
    /* tokio::net::TcpListener::from_std needs an entered Tokio reactor
       (tokio-rs/tokio#7172) and a live one to register with — this runs
       synchronously on the Tauri command thread, outside the async
       runtime, so enter Tauri's managed runtime handle first. */
    let _guard = tauri::async_runtime::handle().inner().enter();
    if let Err(e) = listener.set_nonblocking(true) {
        *last_error().lock().unwrap() = Some(e.to_string());
        return;
    }
    let tokio_listener = match tokio::net::TcpListener::from_std(listener) {
        Ok(l) => l,
        Err(e) => {
            *last_error().lock().unwrap() = Some(e.to_string());
            return;
        }
    };
    *last_error().lock().unwrap() = None;

    let (out_tx, _) = tokio::sync::broadcast::channel::<RemoteMsg>(64);
    let dirty: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let running = Arc::new(AtomicBool::new(true));
    let (kill_tx, kill_rx) = tokio::sync::watch::channel(false);
    *current_out().lock().unwrap() = Some(out_tx.clone());

    let ctx = WsCtx {
        token: Arc::new(token),
        sessions: Arc::new(Sessions::default()),
        out: out_tx.clone(),
        app: app.clone(),
    };
    let router = Router::new().route("/", get(handle_page)).route("/ws", get(handle_ws)).with_state(ctx);

    /* subscribe to pty output/exit now (synchronously), move receivers in */
    let app1 = app.clone();
    let d1 = dirty.clone();
    tauri::async_runtime::spawn(async move {
        let mut rx = app1.state::<crate::pty::PtyManager>().on_term_data();
        loop {
            match rx.recv().await {
                Ok((id, _)) => d1.lock().unwrap().push(id),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return,
            }
        }
    });
    let d2 = dirty.clone();
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut rx = app2.state::<crate::pty::PtyManager>().on_term_exit();
        loop {
            match rx.recv().await {
                Ok((id, _)) => {
                    d2.lock().unwrap().retain(|p| p != &id);
                    broadcast(RemoteMsg::Json(js(&json!({"t": "panes", "panes": pane_list(&app2)}))));
                    broadcast(RemoteMsg::Gone { pane_id: id });
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return,
            }
        }
    });

    /* fixed tick while the server runs; exits on stop */
    let d3 = dirty.clone();
    let run3 = running.clone();
    std::thread::spawn(move || {
        while run3.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(WATCH_TICK_MS));
            push_dirty(&d3);
        }
    });

    let run_serve = running.clone();
    tauri::async_runtime::spawn(async move {
        let _ = axum::serve(tokio_listener, router)
            .with_graceful_shutdown(async move {
                let mut rx = kill_rx;
                while rx.changed().await.is_ok() && !*rx.borrow() {}
            })
            .await;
        run_serve.store(false, Ordering::SeqCst);
    });

    *server().lock().unwrap() = Some(ServerState {
        kill_tx,
        running,
    });
}

pub fn stop_remote() {
    let st = server().lock().unwrap().take();
    let Some(st) = st else { return };
    let _ = st.kill_tx.send(true);
    st.running.store(false, Ordering::SeqCst);
    *current_out().lock().unwrap() = None;
}

// ---------------- cloudflare quick tunnel (public HTTPS) ----------------
// Spawns `cloudflared tunnel --url http://127.0.0.1:<port>` and scrapes
// the assigned https:// trycloudflare.com URL from its stderr. Gives
// phones a real CA-signed cert with zero local trust setup, at the cost
// of routing traffic through Cloudflare's edge instead of staying on
// LAN — the pairing token is still required on every request, so a
// leaked tunnel URL alone can't reach anything.

struct TunnelState {
    child: std::process::Child,
    url: Arc<Mutex<Option<String>>>,
}

static TUNNEL: std::sync::OnceLock<Mutex<Option<TunnelState>>> = std::sync::OnceLock::new();
fn tunnel() -> &'static Mutex<Option<TunnelState>> {
    TUNNEL.get_or_init(|| Mutex::new(None))
}

static TUNNEL_ERROR: std::sync::OnceLock<Mutex<Option<String>>> = std::sync::OnceLock::new();
fn tunnel_error() -> &'static Mutex<Option<String>> {
    TUNNEL_ERROR.get_or_init(|| Mutex::new(None))
}

pub fn tunnel_url() -> Option<String> {
    tunnel().lock().unwrap().as_ref().and_then(|t| t.url.lock().unwrap().clone())
}

pub fn tunnel_running() -> bool {
    tunnel().lock().unwrap().is_some()
}

/* scrapes a `https://...trycloudflare.com` URL out of a cloudflared log line */
fn parse_tunnel_url(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let rest = &line[start..];
    let end = rest.find(|c: char| c.is_whitespace() || c == '|').unwrap_or(rest.len());
    let url = &rest[..end];
    url.contains(".trycloudflare.com").then(|| url.to_string())
}

fn cloudflared_target() -> Option<&'static str> {
    if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("darwin-x86_64")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("darwin-aarch64")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("linux-x86_64")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some("linux-aarch64")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("windows-x86_64")
    } else {
        None
    }
}

fn cloudflared_resource_path(resource_dir: &Path, target: &str, windows: bool) -> std::path::PathBuf {
    resource_dir.join("cloudflared").join(target).join(if windows { "cloudflared.exe" } else { "cloudflared" })
}

/* the gzipped form of the same path: what the bundler actually ships (39.8 MB
   raw -> 20.3 MB gzipped on darwin-x86_64, 37.1 -> 18.5 on darwin-aarch64) */
fn cloudflared_gz_resource_path(resource_dir: &Path, target: &str, windows: bool) -> std::path::PathBuf {
    let mut p = cloudflared_resource_path(resource_dir, target, windows).into_os_string();
    p.push(".gz");
    PathBuf::from(p)
}

/* the bundler declares this resource as `../resources/cloudflared/...` and stores
   `..` components as `_up_`, so the packaged lookup has to go through
   `PathResolver::resolve` (which applies the same rewrite). Joining
   `resource_dir()` directly lands on `Resources/cloudflared/...` — a path that
   never exists in an installed build — and the tunnel then reports
   "cloudflared is not bundled for this platform". */
fn cloudflared_resource_rel(target: &str, windows: bool) -> String {
    format!("../resources/cloudflared/{target}/{}", if windows { "cloudflared.exe" } else { "cloudflared" })
}

fn cloudflared_resource_rel_gz(target: &str, windows: bool) -> String {
    format!("{}.gz", cloudflared_resource_rel(target, windows))
}

/* identifies the exact source file a cached binary was inflated from, so an
   app update that ships a new cloudflared re-inflates instead of reusing the
   stale binary. Length + mtime is enough: this is a build artifact inside the
   signed bundle, not untrusted input. */
fn source_stamp(gz: &Path) -> Option<String> {
    let meta = std::fs::metadata(gz).ok()?;
    let nanos = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Some(format!("{}:{nanos}", meta.len()))
}

/* inflate `gz` to `bin`, chmod +x, and record the source stamp. Writes to a
   process-unique temp file and renames, so a concurrent inflate or a crash
   never leaves a truncated executable behind. */
fn inflate_to(gz: &Path, bin: &Path, stamp: &Path) -> std::io::Result<()> {
    let name = bin.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "cloudflared".to_string());
    let tmp = bin.with_file_name(format!("{name}.{}.tmp", std::process::id()));
    let result = (|| -> std::io::Result<()> {
        let mut input = flate2::read::GzDecoder::new(std::fs::File::open(gz)?);
        let mut out = std::fs::File::create(&tmp)?;
        std::io::copy(&mut input, &mut out)?;
        out.sync_all()?;
        drop(out);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::rename(&tmp, bin)?;
        std::fs::write(stamp, source_stamp(gz).unwrap_or_default())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/* return an executable path for the bundled cloudflared, inflating the shipped
   .gz into the app cache once (and again whenever the bundled copy changes).
   ponytail: dev and installed builds share this cache dir, so alternating
   between them re-inflates each time; give it its own subdir if that ever
   matters. Windows also fails the rename if the installed app is running. */
fn unpack_cloudflared(app: &tauri::AppHandle, target: &str, is_win: bool, gz: &Path) -> Option<String> {
    let name = if is_win { "cloudflared.exe" } else { "cloudflared" };
    let dir = app.path().app_cache_dir().ok()?.join("cloudflared").join(target);
    let bin = dir.join(name);
    let stamp = dir.join("source.meta");
    let source = source_stamp(gz)?;
    if bin.is_file() && std::fs::read_to_string(&stamp).is_ok_and(|s| s == source) {
        return Some(bin.to_string_lossy().into_owned());
    }
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("[bentomux] cloudflared cache dir {}: {e}", dir.display());
        return None;
    }
    if let Err(e) = inflate_to(gz, &bin, &stamp) {
        eprintln!("[bentomux] failed to inflate bundled {}: {e}", gz.display());
        return None;
    }
    Some(bin.to_string_lossy().into_owned())
}

fn bundled_cloudflared(app: &tauri::AppHandle) -> Option<String> {
    let target = cloudflared_target()?;
    let is_win = cfg!(target_os = "windows");
    /* resolve() applies the bundler's `..` -> `_up_` rewrite, so the packaged
       dir has to come from a resolved path rather than resource_dir() */
    let mut packed: Vec<PathBuf> = Vec::new();
    if let Ok(p) = app.path().resolve(cloudflared_resource_rel_gz(target, is_win), BaseDirectory::Resource) {
        packed.push(p);
    }
    /* dev fallback: during `tauri dev` resourceDir is the temp bundle dir, so
       also try the repo layout relative to the executable / cwd. The repo
       holds the same .gz the bundle does (prepare:cloudflared removes the raw
       binary), so dev exercises the inflate path in production too. */
    for base in [
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../resources"),
        std::env::current_dir().unwrap_or_default().join("resources"),
    ] {
        packed.push(cloudflared_gz_resource_path(&base, target, is_win));
    }
    packed
        .into_iter()
        .filter(|p| p.is_file())
        .find_map(|gz| unpack_cloudflared(app, target, is_win, &gz))
}

pub fn start_tunnel(app: &tauri::AppHandle, port: u16) {
    if tunnel_running() {
        return;
    }
    let bin = bundled_cloudflared(app).or_else(|| crate::shell::find_on_path("cloudflared"));
    let Some(bin) = bin else {
        *tunnel_error().lock().unwrap() = Some("cloudflared is not bundled for this platform and was not found on PATH. Install it from https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/ to enable public HTTPS.".to_string());
        return;
    };
    let mut cmd = std::process::Command::new(&bin);
    cmd.args(["tunnel", "--url", &format!("http://127.0.0.1:{port}")])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = match cmd.spawn()
    {
        Ok(c) => c,
        Err(e) => {
            *tunnel_error().lock().unwrap() = Some(format!("failed to start cloudflared: {e}"));
            return;
        }
    };
    *tunnel_error().lock().unwrap() = None;
    let url: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let stderr = child.stderr.take();
    let stdout = child.stdout.take();
    let url2 = url.clone();
    tauri::async_runtime::spawn_blocking(move || {
        use std::io::BufRead;
        // watch both streams; cloudflared prints the URL to stderr on most
        // builds but some wrappers use stdout — check both
        let mut readers: Vec<Box<dyn BufRead + Send>> = Vec::new();
        if let Some(s) = stderr { readers.push(Box::new(std::io::BufReader::new(s))); }
        if let Some(s) = stdout { readers.push(Box::new(std::io::BufReader::new(s))); }
        for mut r in readers {
            // drain in a nested loop so the first stream doesn't block forever
            // on a dead child; each reader runs to EOF independently
            for line in (&mut r).lines().map_while(Result::ok) {
                if let Some(u) = parse_tunnel_url(&line) {
                    let mut cur = url2.lock().unwrap();
                    if cur.is_none() { *cur = Some(u); }
                }
            }
        }
        // if the child exited before printing a URL, surface an error so
        // the panel doesn't stay stuck on "Starting..."
        if url2.lock().unwrap().is_none() {
            let mut err = tunnel_error().lock().unwrap();
            if err.is_none() {
                *err = Some("cloudflared exited without printing a tunnel URL — check network access or try again.".to_string());
            }
        }
    });
    *tunnel().lock().unwrap() = Some(TunnelState { child, url });
}

pub fn stop_tunnel() {
    let st = tunnel().lock().unwrap().take();
    let Some(mut st) = st else { return };
    let _ = st.child.kill();
    let _ = st.child.wait();
}


/* ---------------- settings surface ---------------- */

/* the single pairing URL: the tunnel URL with the token attached; None
   while cloudflared hasn't printed its URL yet (or isn't running) */
fn pairing_url(tunnel: Option<String>, token: &str) -> Vec<String> {
    tunnel.map(|u| vec![format!("{u}/?t={token}")]).unwrap_or_default()
}

#[cfg(test)]
mod pairing_url_tests {
    use super::*;

    #[test]
    fn embeds_token_on_tunnel_url() {
        let urls = pairing_url(
            Some("https://logan-section-yorkshire-petite.trycloudflare.com".into()),
            "tok",
        );
        assert_eq!(
            urls,
            vec!["https://logan-section-yorkshire-petite.trycloudflare.com/?t=tok".to_string()]
        );
    }

    #[test]
    fn empty_while_tunnel_pending() {
        assert!(pairing_url(None, "tok").is_empty());
    }
}

fn qr_svg(url: &str) -> Option<String> {
    let code = qrcode::QrCode::new(url.as_bytes()).ok()?;
    let svg = code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(240, 240)
        .build();
    let b64 = base64::engine::general_purpose::STANDARD.encode(svg.as_bytes());
    Some(format!("data:image/svg+xml;base64,{b64}"))
}

pub fn pairing_info(state: &AppStateManager) -> RemotePairing {
    let prefs = state.get_state().prefs;
    let port = remote_port(&prefs);
    let token = prefs.remote.as_ref().and_then(|r| r.token.clone()).unwrap_or_default();
    let base = tunnel_url();
    let paired = base.clone().map(|u| format!("{u}/?t={token}"));
    let urls = pairing_url(base, &token);
    let qr = urls.first().and_then(|u| qr_svg(u));
    RemotePairing {
        enabled: remote_enabled(&prefs),
        running: remote_running(),
        port,
        token,
        urls,
        qr,
        error: last_error().lock().unwrap().clone(),
        tunnel_url: paired.clone(),
        tunnel_qr: paired.as_deref().and_then(qr_svg),
        tunnel_error: tunnel_error().lock().unwrap().clone(),
    }
}

pub fn set_remote_enabled(app: &tauri::AppHandle, state: &AppStateManager, on: bool) -> RemotePairing {
    state.patch_prefs(|p| {
        let r = p.remote.get_or_insert_with(|| RemotePrefs {
            enabled: None,
            port: None,
            token: None,
        });
        r.enabled = Some(on);
    });
    if on {
        start_remote(app, state);
        /* HTTPS is the only access path — the tunnel always follows the
           local server */
        if remote_running() {
            start_tunnel(app, remote_port(&state.get_state().prefs));
        }
    } else {
        stop_tunnel();
        stop_remote();
    }
    pairing_info(state)
}

/* boot-time restore: Electron's index.ts calls startRemote() when
   prefs.remote.enabled was persisted true from a prior session. The
   Tauri setup hook has no equivalent — without this, `enabled` shows
   "On" from disk while the server/tunnel never actually starts, so the
   panel is stuck on "Starting..." until the user manually flips it. */
pub fn restore_on_startup(app: &tauri::AppHandle, state: &AppStateManager) {
    let prefs = state.get_state().prefs;
    if !remote_enabled(&prefs) {
        return;
    }
    start_remote(app, state);
    if remote_running() {
        start_tunnel(app, remote_port(&state.get_state().prefs));
    }
}

pub fn set_remote_port(app: &tauri::AppHandle, state: &AppStateManager, port: u16) -> RemotePairing {
    state.patch_prefs(|p| {
        let r = p.remote.get_or_insert_with(|| RemotePrefs {
            enabled: None,
            port: None,
            token: None,
        });
        r.port = Some(port);
    });
    if remote_running() {
        let had_tunnel = tunnel_running();
        stop_remote();
        start_remote(app, state);
        if had_tunnel && remote_running() {
            start_tunnel(app, port);
        }
    }
    pairing_info(state)
}

/* ---------------- tests ---------------- */

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_token_is_unique_and_url_safe() {
        let a = random_token();
        let b = random_token();
        assert_ne!(a, b);
        assert_eq!(a.len(), 32); // 24 bytes base64url → 32 chars
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn remote_pane_info_serializes_camel_case() {
        let p = RemotePaneInfo {
            id: "t-1".into(),
            title: "Main".into(),
            workspace: "proj".into(),
            state: Some(crate::runtime::AgentRunState::Working),
            runtime: Some("claude".into()),
        };
        let s = js(&p);
        assert!(s.contains("\"id\":\"t-1\""));
        assert!(s.contains("\"state\":\"working\""));
        assert!(!s.contains('_'));
    }
    #[test]
    fn bundled_resource_path_uses_platform_binary_name() {
        let root = Path::new("/resources");
        assert_eq!(cloudflared_resource_path(root, "darwin-x86_64", false), Path::new("/resources/cloudflared/darwin-x86_64/cloudflared"));
        assert_eq!(cloudflared_resource_path(root, "windows-x86_64", true), Path::new("/resources/cloudflared/windows-x86_64/cloudflared.exe"));
    }

    /* the packaged lookup must stay `../`-relative: the bundler rewrites the
       leading `..` to `_up_` (tauri_utils::resources::resource_relpath)
       because tauri.conf.json points at the repo-level resources/ dir */
    #[test]
    fn bundled_resource_rel_requires_up_prefix_rewrite() {
        assert_eq!(cloudflared_resource_rel("darwin-x86_64", false), "../resources/cloudflared/darwin-x86_64/cloudflared");
        assert_eq!(cloudflared_resource_rel("windows-x86_64", true), "../resources/cloudflared/windows-x86_64/cloudflared.exe");
    }

    /* the bundler ships only the .gz (tauri.conf.json bundles the
       resources/cloudflared directory, which prepare:cloudflared leaves holding
       nothing but the archive), so the packaged lookup has to name the
       compressed file */
    #[test]
    fn bundled_gz_paths_mirror_the_binary_paths() {
        let root = Path::new("/resources");
        assert_eq!(cloudflared_gz_resource_path(root, "darwin-x86_64", false), Path::new("/resources/cloudflared/darwin-x86_64/cloudflared.gz"));
        assert_eq!(cloudflared_gz_resource_path(root, "windows-x86_64", true), Path::new("/resources/cloudflared/windows-x86_64/cloudflared.exe.gz"));
        assert_eq!(cloudflared_resource_rel_gz("linux-aarch64", false), "../resources/cloudflared/linux-aarch64/cloudflared.gz");
    }

    /* the inflate step is what makes the gzipped bundle usable: bytes must
       round-trip, the binary must come out executable, the *.tmp staging file
       must not survive, and the stamp must record this exact source */
    #[test]
    fn inflates_gz_and_stamps_the_source() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("bentomux-inflate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let gz = dir.join("cloudflared.gz");
        let payload = b"#!/bin/sh\necho cloudflared\n";
        let mut enc = flate2::write::GzEncoder::new(std::fs::File::create(&gz).unwrap(), flate2::Compression::default());
        enc.write_all(payload).unwrap();
        enc.finish().unwrap();

        let bin = dir.join("cloudflared");
        let stamp = dir.join("source.meta");
        inflate_to(&gz, &bin, &stamp).unwrap();

        assert_eq!(std::fs::read(&bin).unwrap(), payload);
        assert_eq!(std::fs::read_to_string(&stamp).unwrap(), source_stamp(&gz).unwrap());
        assert!(!dir.join(format!("cloudflared.{}.tmp", std::process::id())).exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&bin).unwrap().permissions().mode() & 0o777, 0o755);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /* a corrupt bundle must fail the inflate instead of leaving a truncated
       executable for spawn to trip over */
    #[test]
    fn failed_inflate_leaves_no_staged_or_target_file() {
        let dir = std::env::temp_dir().join(format!("bentomux-inflate-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let gz = dir.join("cloudflared.gz");
        std::fs::write(&gz, b"not a gzip stream").unwrap();

        let bin = dir.join("cloudflared");
        assert!(inflate_to(&gz, &bin, &dir.join("source.meta")).is_err());
        assert!(!bin.exists());
        assert!(!dir.join(format!("cloudflared.{}.tmp", std::process::id())).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /* the archive the bundler actually ships has to inflate into something
       `start_tunnel` can spawn: this is the only check that runs the real 37-40 MB
       artifact instead of a synthetic payload, and gets skipped (not silently
       passed) when prepare:cloudflared has not run in this checkout */
    #[test]
    fn real_bundled_archive_inflates_to_a_runnable_cloudflared() {
        let Some(target) = cloudflared_target() else { return };
        let is_win = cfg!(target_os = "windows");
        let gz = cloudflared_gz_resource_path(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../resources"), target, is_win);
        if !gz.is_file() {
            eprintln!("skipping: {} not prepared (run `npm run prepare:cloudflared`)", gz.display());
            return;
        }
        let dir = std::env::temp_dir().join(format!("bentomux-real-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join(if is_win { "cloudflared.exe" } else { "cloudflared" });
        inflate_to(&gz, &bin, &dir.join("source.meta")).unwrap();

        // a truncated decode would still return Ok, so size and execution both count
        assert!(std::fs::metadata(&bin).unwrap().len() > 30 * 1024 * 1024, "inflated cloudflared is too small");
        let out = std::process::Command::new(&bin).arg("--version").output().unwrap();
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success(), "cloudflared --version failed: {text}");
        assert!(text.contains("cloudflared version"), "unexpected cloudflared output: {text}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod smoke_socket_conversion {
    /* regression for the fix above: reproduces the exact bind ->
       set_nonblocking -> from_std sequence start_remote runs, outside
       any tokio::main/#[tokio::test] context (mirrors running on the
       Tauri sync-command thread). Panics pre-fix with either
       "Registering a blocking socket..." or "there is no reactor
       running...". */
    #[test]
    fn tcp_listener_converts_without_reactor_panic() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let _guard = tauri::async_runtime::handle().inner().enter();
        listener.set_nonblocking(true).unwrap();
        let _tokio_listener = tokio::net::TcpListener::from_std(listener).unwrap();
    }
}

#[cfg(test)]
mod smoke_tunnel {
    use super::*;

    /* end-to-end proof of the parsing + URL-building logic without
       actually spawning cloudflared: a fixed cloudflared log line goes
       in, the exact URL start_tunnel would capture comes out. */
    #[test]
    fn parses_trycloudflare_url_from_log_line() {
        let line = "2026-09-11T02:30:21Z INF |  https://logan-section-yorkshire-petite.trycloudflare.com                                  |";
        let url = parse_tunnel_url(line).expect("should find url");
        assert_eq!(url, "https://logan-section-yorkshire-petite.trycloudflare.com");
    }

    #[test]
    fn ignores_non_tunnel_urls_in_log_lines() {
        let line = "2026-09-11T02:30:03Z INF Requesting new quick Tunnel on trycloudflare.com...";
        assert!(parse_tunnel_url(line).is_none());
        let line2 = "See https://developers.cloudflare.com/cloudflare-one/ for docs";
        assert!(parse_tunnel_url(line2).is_none());
    }
}

