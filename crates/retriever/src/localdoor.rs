//! The local door: browsers that can't start Retriever's helper through native messaging (Zen and
//! other Firefox-based browsers that read their own folder, Flatpak and Snap browsers) reach it
//! here instead, on 127.0.0.1, while Retriever is open.
//!
//! Every request is the extension's own message, which this door hands to `retriever-host`
//! exactly as a browser would (its length-prefixed JSON on stdin, the answer on stdout): the
//! helper's whole protocol, its checks and its answers, with none of it written twice.
//!
//! Who may knock:
//! - only 127.0.0.1, and only with a Host header naming it (a web page can't reach it through
//!   a name of its own: DNS rebinding);
//! - only from an extension (its Origin is `moz-extension://` or `chrome-extension://`, which a
//!   web page can't send), and only POST;
//! - only with a key the person allowed: the first time, Retriever asks "Connect …?", and the
//!   browser keeps the key it is given. Preferences → Browsers forgets them all.

use crate::model;
use crate::window::UiRc;
use adw::prelude::*;
use gtk::glib;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// The ports it tries, in order (the extension looks on the same ones).
pub const PORTS: [u16; 5] = [46790, 46791, 46792, 46793, 46794];
/// The most a request may be: the helper's messages are small; a page's links can be many.
const MAX_BODY: usize = 8 << 20;
/// How long the helper may take for one answer (a video's look-up is the slowest).
const HOST_WAIT: Duration = Duration::from_secs(90);
/// Retriever's extension, as the helper checks its caller (nmhost: `our_caller`).
const CHROMIUM_ID: &str = "haolajnpdmfpbcpghiinnnighjfhofmb";
const FIREFOX_ID: &str = "{bb6871c1-d6dc-44a5-bde5-1601d6ca0100}";

/// The allowed keys, by key: the browser's name and when.
type Keys = Arc<Mutex<HashMap<String, String>>>;

type Pending = Arc<tokio::sync::Mutex<Option<(std::time::Instant, String, Option<String>)>>>;

/// A browser asking to connect, for the window to ask the person.
struct Ask {
    browser: String,
    answer: tokio::sync::oneshot::Sender<bool>,
}

static KEYS: std::sync::OnceLock<Keys> = std::sync::OnceLock::new();
static ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

fn keys() -> &'static Keys {
    KEYS.get_or_init(|| {
        let saved = std::fs::read(model::browser_keys_path())
            .ok()
            .and_then(|b| serde_json::from_slice::<HashMap<String, String>>(&b).ok())
            .unwrap_or_default();
        Arc::new(Mutex::new(saved))
    })
}

fn save_keys(k: &HashMap<String, String>) {
    if let Ok(bytes) = serde_json::to_vec_pretty(k) {
        let _ = model::write_atomic(&model::browser_keys_path(), &bytes);
    }
}

/// The browsers allowed so far (their names).
pub fn connected() -> Vec<String> {
    let mut v: Vec<String> = keys().lock().unwrap().values().cloned().collect();
    v.sort();
    v.dedup();
    v
}

/// Forgets every browser: each asks again next time.
pub fn forget_all() {
    let mut k = keys().lock().unwrap();
    k.clear();
    save_keys(&k);
}

/// On or off (Preferences): off, the door answers nobody.
pub fn set_on(on: bool) {
    ON.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Opens the door on the first free port, and asks the person when a browser knocks.
pub fn start(ui: &UiRc) {
    set_on(ui.manager.borrow().settings.local_door);
    let (tx, rx) = mpsc::channel::<Ask>();
    crate::manager::rt().spawn(async move {
        let mut listener = None;
        for port in PORTS {
            match TcpListener::bind(("127.0.0.1", port)).await {
                Ok(l) => {
                    listener = Some((l, port));
                    break;
                }
                Err(_) => continue,
            }
        }
        let Some((listener, port)) = listener else {
            eprintln!("retriever: no local door: ports {PORTS:?} all taken");
            return;
        };
        // The last answer: when, to which extension, and its key (None: Don't Allow).
        let pending: Pending = Arc::new(tokio::sync::Mutex::new(None));
        loop {
            let Ok((stream, peer)) = listener.accept().await else {
                continue;
            };
            if !peer.ip().is_loopback() {
                continue;
            }
            let tx = tx.clone();
            let pending = pending.clone();
            tokio::spawn(async move {
                let _ = serve(stream, port, tx, pending).await;
            });
        }
    });
    let ui = ui.clone();
    glib::timeout_add_local(Duration::from_millis(150), move || {
        while let Ok(ask) = rx.try_recv() {
            ask_person(&ui, ask);
        }
        glib::ControlFlow::Continue
    });
}

/// "Connect Zen?": Allow or Don't Allow.
fn ask_person(ui: &UiRc, ask: Ask) {
    ui.window.present();
    let d = adw::AlertDialog::new(
        Some(&format!("Connect {}?", ask.browser)),
        Some(
            "Retriever's extension in this browser wants to send its downloads here. \
             Allow it only if you just installed or opened Retriever's extension.",
        ),
    );
    d.add_responses(&[("deny", "_Don't Allow"), ("allow", "_Allow")]);
    d.set_response_appearance("allow", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("allow"));
    d.set_close_response("deny");
    let answer = std::cell::RefCell::new(Some(ask.answer));
    d.connect_response(None, move |_, r| {
        if let Some(a) = answer.borrow_mut().take() {
            let _ = a.send(r == "allow");
        }
    });
    d.present(Some(&ui.window));
}

struct Request {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

async fn read_request(s: &mut TcpStream) -> Option<Request> {
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        let n = tokio::time::timeout(Duration::from_secs(10), s.read(&mut chunk))
            .await
            .ok()?
            .ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > 32 * 1024 {
            return None;
        }
    };
    let head = std::str::from_utf8(&buf[..head_end]).ok()?;
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_owned();
    let path = first.next()?.to_owned();
    let headers: HashMap<String, String> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    let len: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if len > MAX_BODY {
        return None;
    }
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        let n = tokio::time::timeout(Duration::from_secs(30), s.read(&mut chunk))
            .await
            .ok()?
            .ok()?;
        if n == 0 {
            return None;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(len);
    Some(Request {
        method,
        path,
        headers,
        body,
    })
}

async fn respond(s: &mut TcpStream, code: u16, origin: Option<&str>, body: &[u8]) {
    let reason = match code {
        200 => "OK",
        204 => "No Content",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        _ => "Error",
    };
    let mut head = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n",
        body.len()
    );
    // Firefox may not have granted the extension its host permissions: it then needs these.
    if let Some(o) = origin {
        head.push_str(&format!(
            "Access-Control-Allow-Origin: {o}\r\nAccess-Control-Allow-Methods: POST\r\n\
             Access-Control-Allow-Headers: Content-Type, X-Retriever-Key\r\nVary: Origin\r\n"
        ));
    }
    head.push_str("\r\n");
    let _ = s.write_all(head.as_bytes()).await;
    let _ = s.write_all(body).await;
    let _ = s.shutdown().await;
}

/// An extension's origin, the only callers it answers.
fn extension_origin(h: &HashMap<String, String>) -> Option<&str> {
    h.get("origin")
        .map(String::as_str)
        .filter(|o| o.starts_with("moz-extension://") || o.starts_with("chrome-extension://"))
}

fn host_ok(h: &HashMap<String, String>, port: u16) -> bool {
    h.get("host").is_some_and(|v| {
        v.eq_ignore_ascii_case(&format!("127.0.0.1:{port}"))
            || v.eq_ignore_ascii_case(&format!("localhost:{port}"))
    })
}

async fn serve(
    mut s: TcpStream,
    port: u16,
    ask: mpsc::Sender<Ask>,
    pending: Pending,
) -> Option<()> {
    let req = read_request(&mut s).await?;
    if !ON.load(std::sync::atomic::Ordering::Relaxed) || !host_ok(&req.headers, port) {
        respond(&mut s, 403, None, b"{}").await;
        return Some(());
    }
    let Some(origin) = extension_origin(&req.headers).map(str::to_owned) else {
        respond(&mut s, 403, None, b"{}").await;
        return Some(());
    };
    let o = Some(origin.as_str());
    if req.method == "OPTIONS" {
        respond(&mut s, 204, o, b"").await;
        return Some(());
    }
    if req.method != "POST" {
        respond(&mut s, 404, o, b"{}").await;
        return Some(());
    }
    match req.path.as_str() {
        // "Is this Retriever?": the extension looks for it on PORTS.
        "/retriever" => {
            let body = serde_json::json!({"app": "retriever", "version": env!("CARGO_PKG_VERSION")});
            respond(&mut s, 200, o, body.to_string().as_bytes()).await;
        }
        "/pair" => {
            // One question at a time: a second request waits for the first one's answer.
            let mut last = pending.lock().await;
            // A browser that gave up waiting (closed, restarted) isn't asked about.
            if matches!(s.try_read(&mut [0u8; 1]), Ok(0)) {
                return Some(());
            }
            // The same browser asking again just after an answer: that answer, not a second
            // question (its popup and its pages may all ask at once).
            if let Some((when, ref o2, ref answer)) = *last
                && o2 == &origin
                // Don't Allow holds briefly: asking again later asks the person again.
                && when.elapsed() < Duration::from_secs(if answer.is_some() { 60 } else { 10 })
            {
                match answer {
                    Some(key) if keys().lock().unwrap().contains_key(key) => {
                        let body = serde_json::json!({ "key": key });
                        respond(&mut s, 200, o, body.to_string().as_bytes()).await;
                    }
                    Some(_) => {}
                    None => {
                        respond(&mut s, 403, o, br#"{"why":"denied"}"#).await;
                        return Some(());
                    }
                }
                if answer.is_some() {
                    return Some(());
                }
            }
            let browser = serde_json::from_slice::<serde_json::Value>(&req.body)
                .ok()
                .and_then(|v| v.get("browser")?.as_str().map(str::to_owned))
                .map(|b| b.chars().filter(|c| !c.is_control()).take(40).collect::<String>())
                .filter(|b| !b.trim().is_empty())
                .unwrap_or_else(|| "a Browser".to_owned());
            let (answer, said) = tokio::sync::oneshot::channel();
            ask.send(Ask {
                browser: browser.clone(),
                answer,
            })
            .ok()?;
            if tokio::time::timeout(Duration::from_secs(300), said).await == Ok(Ok(true)) {
                let key = new_key()?;
                {
                    let mut k = keys().lock().unwrap();
                    k.insert(key.clone(), browser);
                    save_keys(&k);
                }
                *last = Some((std::time::Instant::now(), origin.clone(), Some(key.clone())));
                let body = serde_json::json!({ "key": key });
                respond(&mut s, 200, o, body.to_string().as_bytes()).await;
            } else {
                *last = Some((std::time::Instant::now(), origin.clone(), None));
                respond(&mut s, 403, o, br#"{"why":"denied"}"#).await;
            }
        }
        "/host" => {
            let key = req.headers.get("x-retriever-key").cloned().unwrap_or_default();
            if key.is_empty() || !keys().lock().unwrap().contains_key(&key) {
                respond(&mut s, 401, o, b"{}").await;
                return Some(());
            }
            let firefox = origin.starts_with("moz-extension://");
            let body = req.body;
            let reply = tokio::task::spawn_blocking(move || ask_host(&body, firefox))
                .await
                .ok()
                .flatten();
            match reply {
                Some(r) => respond(&mut s, 200, o, &r).await,
                None => respond(&mut s, 502, o, br#"{"ok":false,"why":"host-failed"}"#).await,
            }
        }
        _ => respond(&mut s, 404, o, b"{}").await,
    }
    Some(())
}

/// 32 random bytes, as hex.
fn new_key() -> Option<String> {
    let mut b = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .ok()?
        .read_exact(&mut b)
        .ok()?;
    Some(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// The helper beside this program (the package puts both in /usr/bin), else on PATH.
fn host_program() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| Some(p.parent()?.join("retriever-host")))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| "retriever-host".into())
}

/// One message to `retriever-host`, started as the browser would start it; its answer.
fn ask_host(message: &[u8], firefox: bool) -> Option<Vec<u8>> {
    let mut cmd = std::process::Command::new(host_program());
    if firefox {
        cmd.args(["retriever-local", FIREFOX_ID]);
    } else {
        cmd.arg(format!("chrome-extension://{CHROMIUM_ID}/"));
    }
    let mut child = cmd
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let pid = child.id();
    // A helper that hangs is stopped, as a browser stops it.
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let done = done.clone();
        std::thread::spawn(move || {
            std::thread::sleep(HOST_WAIT);
            if !done.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = std::process::Command::new("kill")
                    .arg(pid.to_string())
                    .status();
            }
        });
    }
    let reply = (|| {
        let mut stdin = child.stdin.take()?;
        stdin
            .write_all(&(message.len() as u32).to_le_bytes())
            .ok()?;
        stdin.write_all(message).ok()?;
        stdin.flush().ok()?;
        let mut out = child.stdout.take()?;
        let mut len = [0u8; 4];
        out.read_exact(&mut len).ok()?;
        let len = u32::from_le_bytes(len) as usize;
        if len > MAX_BODY * 8 {
            return None;
        }
        let mut reply = vec![0u8; len];
        out.read_exact(&mut reply).ok()?;
        drop(stdin);
        Some(reply)
    })();
    done.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = child.kill();
    let _ = child.wait();
    reply
}
