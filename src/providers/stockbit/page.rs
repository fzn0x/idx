//! The local sign-in page (`idx stockbit login --page`): a page on
//! 127.0.0.1 that receives the Stockbit session from the user's own browser,
//! for machines where idx cannot drive one (over SSH, without a display or
//! Chromium) and for accounts that sign in with Google.
//!
//! The user signs in on stockbit.com, ideally in a private window so the
//! everyday browser keeps a session of its own. The page then takes the
//! `credentialStorage` cookie two ways, neither of which involves the
//! password: a bookmarklet the user drags to the bookmarks bar and clicks on
//! the stockbit.com tab (it reads the cookie there and opens this page with
//! the value in the URL *fragment*, which browsers never send to a server or
//! in a referrer), or a paste into a box (DevTools, or the clipboard). The
//! page's script POSTs the value as `text/plain` to its own secret path, and
//! the server hands it to the accept step (`signin.rs`).
//!
//! Safeguards: bound to 127.0.0.1 only; a random 128-bit secret in the path
//! (anything else is 404); the `Host` must be a loopback name (DNS
//! rebinding) and a POST's `Origin` a loopback http origin; at most 16 KiB
//! per POST; `no-store`, `no-referrer`, `nosniff`, no framing and a CSP that
//! allows nothing outside the page; pastes are checked one at a time and
//! none after one is accepted; the server stops after the first acceptance
//! or the timeout. The value is never logged or echoed.

use std::future::Future;
use std::net::Ipv4Addr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use tokio::sync::{watch, Mutex};

use crate::error::{IdxError, Result};

use super::auth::SignInProgress;
use super::signin::{AcceptError, Accepted};
use super::{Stockbit, NAME};

/// The port the page tries first; a free one when it is taken.
pub const DEFAULT_PORT: u16 = 7878;
/// The largest POST accepted (the cookie is a few KiB).
const MAX_BODY: usize = 16 * 1024;

const CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; img-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

type AcceptFuture =
    Pin<Box<dyn Future<Output = std::result::Result<Accepted, AcceptError>> + Send>>;
/// What the page does with a paste.
pub type AcceptFn = Arc<dyn Fn(String) -> AcceptFuture + Send + Sync>;

/// Where the page listens and for how long.
#[derive(Debug, Clone, Copy)]
pub struct PageOptions {
    /// `None`: [`DEFAULT_PORT`], else any free port. `Some(0)`: any free port.
    pub port: Option<u16>,
    pub timeout: Duration,
}

/// `5 minutes`, `90 seconds`, `1 minute`.
pub fn span(d: Duration) -> String {
    let secs = d.as_secs();
    match secs {
        60 => "1 minute".into(),
        s if s >= 120 && s % 60 == 0 => format!("{} minutes", s / 60),
        1 => "1 second".into(),
        s => format!("{s} seconds"),
    }
}

/// `true` when this process runs inside an SSH session.
pub fn over_ssh(env: &dyn Fn(&str) -> Option<String>) -> bool {
    ["SSH_CONNECTION", "SSH_TTY"]
        .iter()
        .any(|k| env(k).is_some_and(|v| !v.trim().is_empty()))
}

/// The forward to run on the machine with the browser:
/// `ssh -N -L <port>:127.0.0.1:<port> <user>@<server>`, the server's address
/// from `SSH_CONNECTION` (`client_ip client_port server_ip server_port`).
pub fn ssh_forward(port: u16, env: &dyn Fn(&str) -> Option<String>) -> String {
    let user = env("USER")
        .or_else(|| env("USERNAME"))
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| "you".into());
    let host = env("SSH_CONNECTION")
        .and_then(|c| c.split_whitespace().nth(2).map(String::from))
        .unwrap_or_else(|| "this-host".into());
    format!("ssh -N -L {port}:127.0.0.1:{port} {user}@{host}")
}

/// 32 hex digits from the OS's random source.
fn secret() -> Result<String> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b)
        .map_err(|e| IdxError::Other(format!("no random source for the sign-in page: {e}")))?;
    Ok(hex::encode(b))
}

/// A bound page, not yet serving.
pub struct PageServer {
    listener: tokio::net::TcpListener,
    secret: String,
    pub url: String,
    pub port: u16,
    pub timeout: Duration,
}

/// Binds 127.0.0.1: `--port N` exactly, else [`DEFAULT_PORT`], else any
/// free port.
pub async fn bind(opts: PageOptions) -> Result<PageServer> {
    use tokio::net::TcpListener;
    let listener = match opts.port {
        Some(p) => TcpListener::bind((Ipv4Addr::LOCALHOST, p))
            .await
            .map_err(|e| {
                IdxError::Usage(format!(
                    "cannot open the sign-in page on 127.0.0.1:{p} ({e}); pick another port with --port"
                ))
            })?,
        None => match TcpListener::bind((Ipv4Addr::LOCALHOST, DEFAULT_PORT)).await {
            Ok(l) => l,
            Err(_) => TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?,
        },
    };
    let port = listener.local_addr()?.port();
    let secret = secret()?;
    Ok(PageServer {
        url: format!("http://127.0.0.1:{port}/login/{secret}"),
        listener,
        secret,
        port,
        timeout: opts.timeout,
    })
}

/// Shared by the handlers of one page.
pub struct PageState {
    accept: AcceptFn,
    /// The accepted session; also the lock that checks pastes one at a time.
    accepted: Mutex<Option<Accepted>>,
    done: watch::Sender<bool>,
    /// How long the page listens, in words.
    span: String,
}

impl PageState {
    pub fn new(accept: AcceptFn, timeout: Duration) -> Arc<Self> {
        Arc::new(PageState {
            accept,
            accepted: Mutex::new(None),
            done: watch::channel(false).0,
            span: span(timeout),
        })
    }

    /// Resolves once a paste was accepted.
    pub async fn accepted(&self) {
        let mut rx = self.done.subscribe();
        let _ = rx.wait_for(|d| *d).await;
    }
}

impl PageServer {
    /// Serves until a paste is accepted or the timeout passes.
    pub async fn wait<F, Fut>(self, accept: F) -> Result<Accepted>
    where
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = std::result::Result<Accepted, AcceptError>> + Send + 'static,
    {
        let PageServer {
            listener,
            secret,
            url,
            timeout,
            ..
        } = self;
        let accept: AcceptFn = Arc::new(move |v| Box::pin(accept(v)));
        let state = PageState::new(accept, timeout);
        let app = router(&secret, state.clone());
        let stop = state.clone();
        let shutdown = async move {
            let _ = tokio::time::timeout(timeout, stop.accepted()).await;
        };
        let mut server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(shutdown)
                .await
        });
        let _ = tokio::time::timeout(timeout, state.accepted()).await;
        // Let the accepted POST's answer reach the browser, but do not wait
        // for idle keep-alive connections.
        if tokio::time::timeout(Duration::from_secs(3), &mut server)
            .await
            .is_err()
        {
            server.abort();
        }
        let accepted = state.accepted.lock().await.clone();
        accepted.ok_or_else(|| {
            IdxError::provider(
                NAME,
                format!(
                    "the sign-in page timed out after {}s; run `idx stockbit login --page` again (its URL was {})",
                    timeout.as_secs(),
                    url
                ),
            )
        })
    }
}

/// The page's routes: `GET` and `POST` on `/login/<secret>`, 404 elsewhere.
pub fn router(secret: &str, state: Arc<PageState>) -> Router {
    let path = format!("/login/{secret}");
    Router::new()
        .route(&path, get(page).post(paste))
        .fallback(|| async { (StatusCode::NOT_FOUND, "not found") })
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .layer(middleware::from_fn(guard))
        .with_state(state)
}

/// `127.0.0.1`, `localhost` or `[::1]`, with any port (an `ssh -L` forward
/// may use another local port).
fn loopback_authority(authority: &str) -> bool {
    let a = authority.trim().to_ascii_lowercase();
    let host = if let Some(rest) = a.strip_prefix('[') {
        match rest.split_once(']') {
            Some((h, tail)) if tail.is_empty() || tail.starts_with(':') => format!("[{h}]"),
            _ => return false,
        }
    } else {
        match a.split_once(':') {
            Some((h, port)) if port.chars().all(|c| c.is_ascii_digit()) => h.to_string(),
            Some(_) => return false,
            None => a.clone(),
        }
    };
    matches!(host.as_str(), "127.0.0.1" | "localhost" | "[::1]")
}

fn loopback_origin(origin: &str) -> bool {
    origin
        .strip_prefix("http://")
        .is_some_and(|rest| !rest.contains('/') && loopback_authority(rest))
}

/// Host and Origin checks, and the security headers on every answer.
async fn guard(req: Request, next: Next) -> Response {
    let allowed = {
        let h = req.headers();
        let get = |name: header::HeaderName| h.get(name).and_then(|v| v.to_str().ok());
        get(header::HOST).is_some_and(loopback_authority)
            && (req.method() != Method::POST || get(header::ORIGIN).is_some_and(loopback_origin))
    };
    let mut res = if allowed {
        next.run(req).await
    } else {
        (StatusCode::FORBIDDEN, "forbidden").into_response()
    };
    let h = res.headers_mut();
    for (k, v) in [
        (header::CACHE_CONTROL, "no-store"),
        (header::REFERRER_POLICY, "no-referrer"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::X_FRAME_OPTIONS, "DENY"),
        (header::CONTENT_SECURITY_POLICY, CSP),
    ] {
        h.insert(k, HeaderValue::from_static(v));
    }
    res
}

async fn page(State(st): State<Arc<PageState>>) -> Response {
    let html = PAGE.replace("__SPAN__", &st.span);
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        Body::from(html),
    )
        .into_response()
}

fn answer(ok: bool, message: &str) -> Response {
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({ "ok": ok, "message": message }).to_string(),
    )
        .into_response()
}

async fn paste(State(st): State<Arc<PageState>>, body: String) -> Response {
    let mut accepted = st.accepted.lock().await;
    if accepted.is_some() {
        return answer(true, "Already connected.");
    }
    match (st.accept)(body).await {
        Ok(a) => {
            *accepted = Some(a);
            st.done.send_replace(true);
            answer(
                true,
                "Connected. Close the private window without signing out (signing out there could end idx's session too).",
            )
        }
        Err(e) => answer(false, &IdxError::from(e).to_string()),
    }
}

// ---------------------------------------------------------------------------
// A page sign-in in the background (MCP)
// ---------------------------------------------------------------------------

struct Slot {
    progress: Arc<std::sync::Mutex<SignInProgress>>,
    until: std::time::Instant,
}

/// One page sign-in per process at a time. Process-wide because the HTTP
/// transport builds one MCP server per session.
static SLOT: std::sync::Mutex<Option<Slot>> = std::sync::Mutex::new(None);

fn snapshot(slot: &Slot) -> SignInProgress {
    let mut p = slot
        .progress
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    if p.state == "waiting" {
        p.expires_in_secs = Some(
            slot.until
                .saturating_duration_since(std::time::Instant::now())
                .as_secs() as i64,
        );
    } else {
        p.expires_in_secs = None;
        p.url = None;
    }
    p
}

/// The latest page sign-in started in this process, if any.
pub fn progress() -> Option<SignInProgress> {
    SLOT.lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(snapshot)
}

/// Opens the page and returns at once with its URL; a background task
/// serves it and records the outcome, which [`progress`] reports. A page
/// still waiting is returned instead of a second one.
pub async fn start_in_background(
    sb: Arc<Stockbit>,
    opts: PageOptions,
    env: &(dyn Fn(&str) -> Option<String> + Sync),
) -> Result<SignInProgress> {
    {
        let slot = SLOT.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = slot.as_ref() {
            let p = snapshot(s);
            if p.state == "waiting" && std::time::Instant::now() < s.until {
                return Ok(SignInProgress {
                    already_open: true,
                    ..p
                });
            }
        }
    }
    let server = bind(opts).await?;
    let progress = Arc::new(std::sync::Mutex::new(SignInProgress {
        state: "waiting".into(),
        method: "page".into(),
        url: Some(server.url.clone()),
        port: Some(server.port),
        ssh_forward: over_ssh(env).then(|| ssh_forward(server.port, env)),
        expires_in_secs: Some(opts.timeout.as_secs() as i64),
        ..Default::default()
    }));
    let slot = Slot {
        progress: progress.clone(),
        until: std::time::Instant::now() + opts.timeout,
    };
    let first = snapshot(&slot);
    *SLOT.lock().unwrap_or_else(|e| e.into_inner()) = Some(slot);
    let notes = progress.clone();
    let accept = move |v: String| {
        let sb = sb.clone();
        let notes = notes.clone();
        async move {
            let r = sb.accept_session(&v).await;
            let mut p = notes.lock().unwrap_or_else(|e| e.into_inner());
            match &r {
                Ok(a) => p.warning = a.warning.clone(),
                Err(e) => p.warning = Some(describe(e)),
            }
            r
        }
    };
    tokio::spawn(async move {
        let outcome = server.wait(accept).await;
        let mut p = progress.lock().unwrap_or_else(|e| e.into_inner());
        match outcome {
            Ok(_) => p.state = "signed_in".into(),
            Err(e) => {
                p.state = "failed".into();
                p.error = Some(e.to_string());
            }
        }
    });
    Ok(first)
}

/// A refused or failed paste in words, never the value.
pub fn describe(e: &AcceptError) -> String {
    match e {
        AcceptError::NotASession => "a paste was not a Stockbit session".into(),
        AcceptError::Refused(why) => format!("Stockbit refused a pasted session ({why})"),
        AcceptError::Failed(e) => format!("a pasted session could not be checked ({e})"),
    }
}

/// The page. No external resources; the bookmarklet is built by the script
/// from this page's own address, so it is right behind a forward to another
/// local port too. It reads `credentialStorage` only on a stockbit.com tab
/// and passes it in the URL fragment.
const PAGE: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="color-scheme" content="light dark">
<meta name="referrer" content="no-referrer">
<title>Sign in to Stockbit for idx</title>
<style>
:root{--bg:#f5f6f8;--card:#fff;--ink:#14161b;--muted:#5d6472;--line:#dfe3ea;--accent:#1d5fd1;--accent-ink:#fff;--ok:#0d7a45;--err:#b3261e;--code:#eef1f5}
@media (prefers-color-scheme:dark){:root{--bg:#0e1013;--card:#171a1f;--ink:#e9ebef;--muted:#9aa2b1;--line:#2a2f38;--accent:#7aa7ff;--accent-ink:#0e1013;--ok:#5fd39a;--err:#ff8f86;--code:#20242b}}
*{box-sizing:border-box}
body{margin:0;padding:24px 16px;background:var(--bg);color:var(--ink);font:15px/1.6 system-ui,-apple-system,"Segoe UI",Roboto,sans-serif}
main{max-width:640px;margin:0 auto;background:var(--card);border:1px solid var(--line);border-radius:14px;padding:24px}
h1{font-size:21px;line-height:1.3;margin:0 0 6px}
.lede{color:var(--muted);margin:0 0 20px}
ol{margin:0;padding:0;list-style:none;display:grid;gap:14px}
li{border:1px solid var(--line);border-radius:10px;padding:14px 16px}
h2{font-size:15px;margin:0 0 4px}
p{margin:0 0 10px}
li p:last-child{margin-bottom:0}
.muted{color:var(--muted);font-size:14px}
a{color:var(--accent)}
.btn{display:inline-block;border:1px solid var(--accent);border-radius:8px;padding:8px 14px;font:inherit;font-weight:600;background:var(--accent);color:var(--accent-ink);text-decoration:none;cursor:pointer}
.btn.ghost{background:transparent;color:var(--accent)}
.mark{cursor:grab}
textarea{width:100%;min-height:80px;margin:8px 0;border:1px solid var(--line);border-radius:8px;background:var(--bg);color:var(--ink);padding:8px;font:13px/1.4 ui-monospace,Menlo,Consolas,monospace;resize:vertical}
.row{display:flex;flex-wrap:wrap;gap:8px}
code{font:13px ui-monospace,Menlo,Consolas,monospace;background:var(--code);border-radius:4px;padding:0 4px;overflow-wrap:anywhere}
#status{margin:16px 0 0;font-weight:600;min-height:1.6em}
#status.ok{color:var(--ok)}#status.err{color:var(--err)}#status.wait{color:var(--muted)}
#done{display:none}
footer{margin-top:20px;padding-top:14px;border-top:1px solid var(--line);color:var(--muted);font-size:13px}
</style>
</head>
<body>
<main>
<section id="steps">
<h1>Sign in to Stockbit for idx</h1>
<p class="lede">idx never asks for your password. You sign in on stockbit.com; this page takes only the session that sign-in leaves behind.</p>
<ol>
<li>
<h2>1. Sign in to Stockbit in a private window</h2>
<p>Open a private (incognito) window and sign in at <a href="https://stockbit.com/login" target="_blank" rel="noreferrer noopener">stockbit.com/login</a>.</p>
<p class="muted">idx then holds a session of its own, and your everyday browser stays signed in: two holders of one session would log each other out.</p>
</li>
<li>
<h2>2. Send the session to idx</h2>
<p>Drag <a id="send" class="btn mark" href="#" draggable="true">Send to idx</a> to your bookmarks bar (Ctrl+Shift+B or Cmd+Shift+B shows it), then click that bookmark on the stockbit.com tab.</p>
<p class="muted">The bookmark only runs on stockbit.com. It opens this page with the session in the address fragment, which never leaves your browser except to this page on your own machine. A bookmark from an earlier run will not work: drag it again.</p>
</li>
<li>
<h2>Or paste the cookie</h2>
<p class="muted">In the private window: DevTools (F12) &gt; Application &gt; Cookies &gt; https://stockbit.com, then copy the value of <code>credentialStorage</code>.</p>
<textarea id="paste" placeholder="credentialStorage value" spellcheck="false" autocomplete="off"></textarea>
<div class="row"><button id="connect" class="btn" type="button">Connect</button><button id="clip" class="btn ghost" type="button">Paste from clipboard</button></div>
</li>
</ol>
</section>
<section id="done">
<h1>Connected</h1>
<p>Stockbit accepted the session and idx stored it. Close the private window without signing out: signing out there could end idx's session too.</p>
</section>
<p id="status" role="status" aria-live="polite"></p>
<footer>idx stores only the two tokens Stockbit issues, in a file only you can read, and never sees your password. This page stops listening after __SPAN__.</footer>
</main>
<script>
(function () {
  var here = location.origin + location.pathname;
  var status = document.getElementById("status");
  function show(text, kind) { status.textContent = text; status.className = kind || ""; }
  var code = "(()=>{const h=location.hostname;if(h!=='stockbit.com'&&!h.endsWith('.stockbit.com')){alert('Click this bookmark on a stockbit.com tab.');return}" +
    "const c=document.cookie.split(';').map(s=>s.trim()).find(s=>s.indexOf('credentialStorage=')===0);" +
    "if(!c){alert('No Stockbit session in this tab: sign in first.');return}" +
    "window.open(" + JSON.stringify(here) + "+String.fromCharCode(35)+'t='+encodeURIComponent(c.slice(18)))})()";
  var mark = document.getElementById("send");
  mark.setAttribute("href", "javascript:" + code);
  mark.addEventListener("click", function (e) {
    e.preventDefault();
    show("Drag this button to your bookmarks bar, then click the bookmark on the stockbit.com tab.", "");
  });
  var busy = false;
  function send(value) {
    value = (value || "").trim();
    if (!value) { show("Paste the cookie value first.", "err"); return; }
    if (busy) return;
    busy = true;
    show("Checking the session with Stockbit...", "wait");
    fetch(location.pathname, { method: "POST", headers: { "Content-Type": "text/plain" }, body: value })
      .then(function (r) { return r.json().catch(function () { return { ok: false, message: "HTTP " + r.status }; }); })
      .then(function (j) {
        busy = false;
        if (j.ok) {
          document.getElementById("steps").style.display = "none";
          document.getElementById("done").style.display = "block";
          show(j.message, "ok");
        } else {
          show(j.message, "err");
        }
      })
      .catch(function () {
        busy = false;
        show("idx is no longer listening (the page timed out or the sign-in finished). Run idx stockbit login --page again.", "err");
      });
  }
  document.getElementById("connect").addEventListener("click", function () {
    send(document.getElementById("paste").value);
  });
  document.getElementById("clip").addEventListener("click", function () {
    if (!navigator.clipboard || !navigator.clipboard.readText) {
      show("This browser cannot read the clipboard here; paste into the box instead.", "err");
      return;
    }
    navigator.clipboard.readText().then(send, function () {
      show("The browser did not allow reading the clipboard; paste into the box instead.", "err");
    });
  });
  if (location.hash.indexOf("#t=") === 0) {
    var v = location.hash.slice(3);
    history.replaceState(null, "", location.pathname);
    try { v = decodeURIComponent(v); } catch (e) {}
    send(v);
  }
})();
</script>
</body>
</html>
"##;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::stockbit::auth::{Status, TokenInfo};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    const SECRET: &str = "0123456789abcdef0123456789abcdef";

    fn accepted() -> Accepted {
        let t = TokenInfo {
            present: true,
            expires_at: None,
            expires_in_secs: None,
        };
        Accepted {
            status: Status {
                logged_in: true,
                renewable: true,
                expired: false,
                reason: None,
                access: t.clone(),
                refresh: t,
                session_file: "s.json".into(),
                sign_in: None,
            },
            unchanged: false,
            warning: None,
        }
    }

    /// A page whose accept step takes "good" and refuses anything else,
    /// counting its calls.
    fn app() -> (Router, Arc<PageState>, Arc<std::sync::atomic::AtomicUsize>) {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let n = calls.clone();
        let accept: AcceptFn = Arc::new(move |v: String| {
            n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Box::pin(async move {
                if v == "good" {
                    Ok(accepted())
                } else {
                    Err(AcceptError::Refused("HTTP 401".into()))
                }
            })
        });
        let st = PageState::new(accept, Duration::from_secs(300));
        (router(SECRET, st.clone()), st, calls)
    }

    fn req(method: &str, path: &str, host: &str, origin: Option<&str>, body: &str) -> Request {
        let mut b = axum::http::Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, host);
        if let Some(o) = origin {
            b = b.header(header::ORIGIN, o);
        }
        b.body(Body::from(body.to_string())).unwrap()
    }

    async fn text(res: Response) -> String {
        String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap()
    }

    fn path() -> String {
        format!("/login/{SECRET}")
    }

    #[tokio::test]
    async fn the_page_is_served_only_on_its_secret_path() {
        let (app, _st, _) = app();
        let res = app
            .clone()
            .oneshot(req("GET", &path(), "127.0.0.1:7878", None, ""))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let h = res.headers().clone();
        assert_eq!(h[header::CACHE_CONTROL], "no-store");
        assert_eq!(h[header::REFERRER_POLICY], "no-referrer");
        assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(h[header::X_FRAME_OPTIONS], "DENY");
        assert!(h[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .starts_with("default-src 'none'"));
        let html = text(res).await;
        for needle in [
            "Send to idx",
            "private window",
            "Paste from clipboard",
            "never asks for your password",
            "without signing out",
        ] {
            assert!(html.contains(needle), "{needle}");
        }
        assert!(html.contains("after 5 minutes"));
        assert!(
            !html.contains("<script src") && !html.contains("<link") && !html.contains("@import")
        );
        assert!(!html.contains('\u{2014}'), "no em dash");
        assert!(!html.contains(SECRET), "the page never prints its secret");
        for link in html
            .match_indices("href=\"http")
            .map(|(i, _)| &html[i + 6..])
        {
            assert!(link.starts_with("https://stockbit.com/"), "{link:.40}");
        }
        for (method, p) in [
            ("GET", "/"),
            ("GET", "/login/0123"),
            ("GET", "/login/0123456789abcdef0123456789abcdee"),
            ("POST", "/login/x"),
        ] {
            let res = app
                .clone()
                .oneshot(req(method, p, "127.0.0.1", Some("http://127.0.0.1"), ""))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::NOT_FOUND, "{method} {p}");
            assert_eq!(res.headers()[header::CACHE_CONTROL], "no-store");
            assert!(!text(res).await.contains(SECRET));
        }
    }

    #[tokio::test]
    async fn foreign_hosts_and_origins_are_refused() {
        let (app, _st, calls) = app();
        for (host, origin) in [
            ("evil.test", Some("http://evil.test")),
            ("127.0.0.1.evil.test", Some("http://127.0.0.1")),
            ("127.0.0.1:7878", None),
            ("127.0.0.1:7878", Some("https://stockbit.com")),
            ("127.0.0.1:7878", Some("http://evil.test:7878")),
            ("127.0.0.1:7878", Some("null")),
            ("localhost:7878", Some("https://localhost:7878")),
        ] {
            let res = app
                .clone()
                .oneshot(req("POST", &path(), host, origin, "good"))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::FORBIDDEN, "{host} {origin:?}");
        }
        let res = app
            .clone()
            .oneshot(req("GET", &path(), "evil.test", None, ""))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        for ok in [
            "127.0.0.1",
            "127.0.0.1:9999",
            "localhost:7878",
            "[::1]:7878",
            "LOCALHOST",
        ] {
            assert!(loopback_authority(ok), "{ok}");
        }
        for no in ["", "[::1]x", "127.0.0.1:x", "evil.test:7878", "127.0.0.2"] {
            assert!(!loopback_authority(no), "{no}");
        }
    }

    #[tokio::test]
    async fn pastes_are_checked_and_one_is_accepted() {
        let (app, st, calls) = app();
        let post = |body: &str| {
            req(
                "POST",
                &path(),
                "127.0.0.1:7878",
                Some("http://127.0.0.1:7878"),
                body,
            )
        };
        let res = app
            .clone()
            .oneshot(post(&"x".repeat(17 * 1024)))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let res = app.clone().oneshot(post("bad")).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let j: serde_json::Value = serde_json::from_str(&text(res).await).unwrap();
        assert_eq!(j["ok"], false);
        assert!(j["message"]
            .as_str()
            .unwrap()
            .contains("Stockbit refused this session"));
        assert!(st.accepted.lock().await.is_none());
        let res = app.clone().oneshot(post("good")).await.unwrap();
        let j: serde_json::Value = serde_json::from_str(&text(res).await).unwrap();
        assert_eq!(j["ok"], true);
        assert!(j["message"].as_str().unwrap().starts_with("Connected."));
        tokio::time::timeout(Duration::from_secs(1), st.accepted())
            .await
            .expect("acceptance is signalled");
        // Later pastes are not checked again.
        let res = app.clone().oneshot(post("bad")).await.unwrap();
        let j: serde_json::Value = serde_json::from_str(&text(res).await).unwrap();
        assert_eq!(
            (j["ok"].as_bool(), j["message"].as_str()),
            (Some(true), Some("Already connected."))
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_bound_page_times_out_naming_its_url() {
        let server = bind(PageOptions {
            port: Some(0),
            timeout: Duration::from_millis(150),
        })
        .await
        .unwrap();
        assert_ne!(server.port, 0);
        let url = server.url.clone();
        assert!(url.starts_with(&format!("http://127.0.0.1:{}/login/", server.port)));
        assert_eq!(url.rsplit('/').next().unwrap().len(), 32);
        let e = server
            .wait(|_v: String| async { Ok(accepted()) })
            .await
            .unwrap_err();
        assert!(
            e.to_string().contains("timed out") && e.to_string().contains(&url),
            "{e}"
        );
    }

    #[tokio::test]
    async fn a_paste_over_a_real_socket_ends_the_wait() {
        let server = bind(PageOptions {
            port: Some(0),
            timeout: Duration::from_secs(20),
        })
        .await
        .unwrap();
        let (url, port) = (server.url.clone(), server.port);
        let waiting = tokio::spawn(server.wait(|v: String| async move {
            if v == "good" {
                Ok(accepted())
            } else {
                Err(AcceptError::NotASession)
            }
        }));
        let client = reqwest::Client::new();
        let origin = format!("http://127.0.0.1:{port}");
        let post = |body: &'static str| {
            client
                .post(&url)
                .header("origin", &origin)
                .header("content-type", "text/plain")
                .body(body)
                .send()
        };
        let page = client.get(&url).send().await.unwrap();
        assert_eq!(page.status(), 200);
        let r: serde_json::Value =
            serde_json::from_str(&post("junk").await.unwrap().text().await.unwrap()).unwrap();
        assert_eq!(r["ok"], false);
        assert!(r["message"]
            .as_str()
            .unwrap()
            .contains("not a Stockbit session"));
        let r: serde_json::Value =
            serde_json::from_str(&post("good").await.unwrap().text().await.unwrap()).unwrap();
        assert_eq!(r["ok"], true);
        let got = tokio::time::timeout(Duration::from_secs(10), waiting)
            .await
            .expect("the wait ends soon after acceptance")
            .unwrap()
            .unwrap();
        assert!(got.status.logged_in);
    }

    #[tokio::test]
    async fn a_background_page_reports_its_progress() {
        let dir = tempfile::tempdir().unwrap();
        let fetcher = Arc::new(crate::http::Fetcher::new(
            Arc::new(crate::http::FixtureBackend::new()),
            crate::cache::Cache::new(dir.path().join("cache")),
            crate::http::CacheMode::Normal,
        ));
        let store = Arc::new(crate::providers::stockbit::SessionStore::new(
            dir.path().join("session.json"),
            None,
            None,
        ));
        let sb = Arc::new(Stockbit::new(fetcher, Default::default(), store));
        let ssh = |k: &str| (k == "SSH_CONNECTION").then(|| "1.2.3.4 5 10.0.0.9 22".to_string());
        let opts = PageOptions {
            port: Some(0),
            timeout: Duration::from_millis(400),
        };
        let first = start_in_background(sb.clone(), opts, &ssh).await.unwrap();
        assert_eq!(
            (first.state.as_str(), first.method.as_str()),
            ("waiting", "page")
        );
        assert!(!first.already_open);
        let url = first.url.clone().unwrap();
        assert!(url.starts_with("http://127.0.0.1:"));
        assert!(first
            .ssh_forward
            .as_deref()
            .is_some_and(|f| f.ends_with("@10.0.0.9")));
        // A second start while the first waits returns the same page.
        let again = start_in_background(sb.clone(), opts, &ssh).await.unwrap();
        assert!(again.already_open);
        assert_eq!(again.url.as_deref(), Some(url.as_str()));
        assert_eq!(progress().unwrap().state, "waiting");
        // Nobody sends a session: it fails with the timeout, and the dead
        // URL is no longer shown.
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let done = progress().unwrap();
        assert_eq!(done.state, "failed");
        assert!(done
            .error
            .as_deref()
            .is_some_and(|e| e.contains("timed out")));
        assert_eq!(done.url, None);
    }

    #[tokio::test]
    async fn a_pinned_port_that_is_taken_is_a_usage_error() {
        let held = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = held.local_addr().unwrap().port();
        let e = bind(PageOptions {
            port: Some(port),
            timeout: Duration::from_secs(1),
        })
        .await
        .err()
        .unwrap();
        assert_eq!(e.exit_code(), 2);
        assert!(e.to_string().contains(&format!("127.0.0.1:{port}")), "{e}");
    }

    #[test]
    fn spans_read_as_words() {
        for (secs, words) in [
            (300, "5 minutes"),
            (60, "1 minute"),
            (90, "90 seconds"),
            (12, "12 seconds"),
            (1, "1 second"),
        ] {
            assert_eq!(span(Duration::from_secs(secs)), words);
        }
    }

    #[test]
    fn the_ssh_forward_names_the_server() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| v.to_string())
            }
        };
        let e = env(&[
            ("SSH_CONNECTION", "10.0.0.5 51234 10.0.0.9 22"),
            ("USER", "dev"),
        ]);
        assert_eq!(
            ssh_forward(7878, &e),
            "ssh -N -L 7878:127.0.0.1:7878 dev@10.0.0.9"
        );
        assert!(over_ssh(&e));
        let e = env(&[("USERNAME", "win")]);
        assert_eq!(
            ssh_forward(9000, &e),
            "ssh -N -L 9000:127.0.0.1:9000 win@this-host"
        );
        assert!(!over_ssh(&e));
        let e = env(&[("SSH_TTY", "/dev/pts/1")]);
        assert_eq!(ssh_forward(1, &e), "ssh -N -L 1:127.0.0.1:1 you@this-host");
        assert!(over_ssh(&e));
    }
}
