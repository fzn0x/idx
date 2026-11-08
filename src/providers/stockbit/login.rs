//! Browser sign-in: launch a Chromium-family browser on Stockbit's own sign-in
//! page with a profile only idx uses, and read the session cookie that
//! signing in leaves (`credentialStorage`) over the Chrome DevTools Protocol.
//! The human does the sign-in (password, OTP, captcha) on stockbit.com.
//!
//! idx only ever polls cookies (`Storage.getCookies`): it intercepts no
//! request or response, so the password never passes through idx's process.
//! Every sign-in path Stockbit has (password, Google or Apple, OTP, recovery,
//! a refresh) ends in that cookie. Each new cookie value goes to the accept
//! step (`signin.rs`), which checks it with Stockbit before storing it; a
//! value Stockbit refuses (say a profile's stale cookie from an earlier
//! sign-in, whose refresh token idx has since rotated) is reported once and
//! ignored until it changes. Once a session is taken, idx closes Stockbit's
//! tabs, deletes the cookie from its profile (so a later run cannot pick up
//! a pair idx has rotated) and closes the browser.
//!
//! The wait itself ([`wait_for_session`]) runs against the [`DevTools`] trait
//! and is unit-tested with a fake; the Chrome side needs a real browser and
//! is a manual check (see `docs/stockbit.md`).

use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::{oneshot, Mutex};

use crate::error::{IdxError, Result};

use super::auth::{digest, session_from_credential_blob};
use super::signin::{AcceptError, Accepted};

pub struct LoginOptions {
    /// Browser executable; discovered when `None`.
    pub browser: Option<PathBuf>,
    /// Profile only idx uses; keeps Stockbit's device cookies between runs,
    /// not the session (removed after each sign-in).
    pub profile_dir: PathBuf,
    /// Overall ceiling on the sign-in.
    pub timeout: Duration,
}

/// The Stockbit web sign-in page the browser opens.
const LOGIN_URL: &str = "https://stockbit.com/login";
/// The cookie stockbit.com keeps the session in.
const COOKIE: &str = "credentialStorage";

/// One cookie as DevTools reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
}

/// What the wait needs from a browser.
#[async_trait]
pub trait DevTools: Send + Sync {
    /// Every cookie in the browser profile.
    async fn cookies(&self) -> Result<Vec<Cookie>>;
    /// Open tabs (page targets); an error once the browser is gone.
    async fn pages(&self) -> Result<usize>;
    /// Closes Stockbit's tabs (its app writes the cookie back from memory),
    /// deletes `c` from the profile and confirms it is gone.
    async fn remove_cookie(&self, c: &Cookie) -> Result<()>;
    /// Closes the browser.
    async fn close(&self);
}

/// The wait's clock.
#[derive(Debug, Clone, Copy)]
pub struct WaitTimes {
    /// How often cookies are read.
    pub poll: Duration,
    /// How long before a value whose check failed (not refused) is tried again.
    pub retry: Duration,
    /// When to give up.
    pub timeout: Duration,
}

impl WaitTimes {
    pub fn new(timeout: Duration) -> Self {
        WaitTimes {
            poll: Duration::from_secs(1),
            retry: Duration::from_secs(5),
            timeout,
        }
    }
}

/// `stockbit.com` or a subdomain, from a cookie's domain (a leading dot
/// means the cookie is shared with subdomains). Parsed, not `contains`.
pub fn stockbit_domain(domain: &str) -> bool {
    let d = domain.trim().trim_start_matches('.').to_ascii_lowercase();
    d == "stockbit.com" || d.ends_with(".stockbit.com")
}

/// The session cookies worth checking: stockbit.com's `credentialStorage`
/// holding a refresh token (a signed-out page leaves the cookie without one).
fn candidates(cookies: Vec<Cookie>) -> Vec<Cookie> {
    cookies
        .into_iter()
        .filter(|c| c.name == COOKIE && stockbit_domain(&c.domain))
        .filter(|c| {
            session_from_credential_blob(&c.value).is_some_and(|s| !s.refresh.trim().is_empty())
        })
        .collect()
}

/// Polls `dt` for a session cookie until `accept` takes one, the browser is
/// closed, or the time runs out. `say` prints progress for the user.
pub async fn wait_for_session<D, F, Fut>(
    dt: &D,
    times: WaitTimes,
    mut accept: F,
    say: &(dyn Fn(&str) + Send + Sync),
) -> Result<Accepted>
where
    D: DevTools + ?Sized,
    F: FnMut(String) -> Fut,
    Fut: Future<Output = std::result::Result<Accepted, AcceptError>>,
{
    let deadline = tokio::time::Instant::now() + times.timeout;
    let mut refused: Vec<[u8; 32]> = Vec::new();
    let mut retry: Option<([u8; 32], tokio::time::Instant)> = None;
    let mut seen_page = false;
    let mut empty = 0;
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(IdxError::provider(
                super::NAME,
                format!(
                    "no Stockbit sign-in within {}s; run `idx stockbit login` again",
                    times.timeout.as_secs()
                ),
            ));
        }
        match dt.pages().await {
            Ok(0) if seen_page => {
                empty += 1;
                if empty >= 2 {
                    return Err(closed());
                }
            }
            Ok(0) => {}
            Ok(_) => {
                seen_page = true;
                empty = 0;
            }
            Err(_) => return Err(closed()),
        }
        let now = tokio::time::Instant::now();
        for c in candidates(dt.cookies().await.unwrap_or_default()) {
            let key = digest(&c.value);
            if refused.contains(&key) || retry.is_some_and(|(k, at)| k == key && now < at) {
                continue;
            }
            match accept(c.value.clone()).await {
                Ok(accepted) => {
                    if let Err(e) = dt.remove_cookie(&c).await {
                        say(&format!(
                            "warning: could not delete the session cookie from idx's browser profile ({e}); deleting the profile folder removes it"
                        ));
                    }
                    dt.close().await;
                    return Ok(accepted);
                }
                Err(AcceptError::Refused(why)) => {
                    say(&format!(
                        "Stockbit refused the session in idx's browser window ({why}); sign in again there"
                    ));
                    refused.push(key);
                }
                Err(AcceptError::NotASession) => refused.push(key),
                Err(AcceptError::Failed(e)) if e.exit_code() == 5 => return Err(e),
                Err(AcceptError::Failed(e)) => {
                    say(&format!(
                        "could not check the session with Stockbit ({e}); trying again in {}s",
                        times.retry.as_secs()
                    ));
                    retry = Some((key, now + times.retry));
                }
            }
            break;
        }
        tokio::time::sleep(times.poll).await;
    }
}

fn closed() -> IdxError {
    IdxError::provider(
        super::NAME,
        "the browser window was closed before a Stockbit sign-in",
    )
}

/// Launches the browser on Stockbit's sign-in page and waits for a session
/// that `accept` takes.
pub async fn capture<F, Fut>(
    opts: &LoginOptions,
    accept: F,
    say: &(dyn Fn(&str) + Send + Sync),
) -> Result<Accepted>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = std::result::Result<Accepted, AcceptError>>,
{
    let bin = match &opts.browser {
        Some(b) => b.clone(),
        None => find_browser().ok_or_else(|| {
            IdxError::Usage(
                "no Chromium-family browser found; set IDX_BROWSER=/path/to/chrome, or use \
                 `idx stockbit login --manual` to paste the session"
                    .into(),
            )
        })?,
    };
    std::fs::create_dir_all(&opts.profile_dir)?;
    let port_file = opts.profile_dir.join("DevToolsActivePort");
    let _ = std::fs::remove_file(&port_file);
    let mut child = spawn_browser(&bin, &opts.profile_dir)?;
    let guard = ChildGuard(&mut child);
    let run = async {
        let (port, path) = wait_for_port_file(&port_file, Duration::from_secs(20)).await?;
        let cdp = Cdp::connect(&format!("ws://127.0.0.1:{port}{path}")).await?;
        wait_for_session(&Chrome(cdp), WaitTimes::new(opts.timeout), accept, say).await
    };
    let result = tokio::time::timeout(opts.timeout + Duration::from_secs(25), run).await;
    drop(guard); // kills the browser whatever happened
    result.unwrap_or_else(|_| {
        Err(IdxError::provider(
            super::NAME,
            "no Stockbit sign-in before the timeout; run `idx stockbit login` again",
        ))
    })
}

// ---------------------------------------------------------------------------
// Browser process
// ---------------------------------------------------------------------------

fn spawn_browser(bin: &Path, profile: &Path) -> Result<Child> {
    Command::new(bin)
        // Port 0: the browser picks a free port and writes it, with the
        // browser endpoint's path, to <profile>/DevToolsActivePort.
        .arg("--remote-debugging-port=0")
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-extensions")
        .arg("--new-window")
        .arg(LOGIN_URL)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| IdxError::provider(super::NAME, format!("could not launch the browser: {e}")))
}

/// Kills the browser when the sign-in finishes, however it finishes.
struct ChildGuard<'a>(&'a mut Child);
impl Drop for ChildGuard<'_> {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// `"<port>\n<path>"` from `DevToolsActivePort`.
pub fn parse_port_file(text: &str) -> Option<(u16, String)> {
    let mut lines = text.lines();
    let port: u16 = lines.next()?.trim().parse().ok().filter(|p| *p > 0)?;
    let path = lines.next()?.trim();
    path.starts_with('/').then(|| (port, path.to_string()))
}

async fn wait_for_port_file(file: &Path, timeout: Duration) -> Result<(u16, String)> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(found) = std::fs::read_to_string(file)
            .ok()
            .and_then(|t| parse_port_file(&t))
        {
            return Ok(found);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(IdxError::provider(
                super::NAME,
                "the browser never opened its DevTools port; close any window using idx's Stockbit profile and retry",
            ));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Common install locations for Chromium-family browsers, plus `IDX_BROWSER` /
/// `STOCKBIT_BROWSER` overrides. Firefox and Safari do not speak CDP and are
/// not listed.
pub fn find_browser() -> Option<PathBuf> {
    for key in ["IDX_BROWSER", "STOCKBIT_BROWSER"] {
        if let Some(p) = std::env::var_os(key) {
            let p = PathBuf::from(p);
            if p.exists() {
                return Some(p);
            }
        }
    }
    let candidates: &[&str] = if cfg!(windows) {
        &[
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
        ]
    } else if cfg!(target_os = "macos") {
        &[
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ]
    } else {
        &[
            "/usr/bin/google-chrome",
            "/usr/bin/google-chrome-stable",
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
            "/usr/bin/microsoft-edge",
            "/usr/bin/brave-browser",
        ]
    };
    candidates.iter().map(PathBuf::from).find(|p| p.exists())
}

// ---------------------------------------------------------------------------
// Chrome over DevTools
// ---------------------------------------------------------------------------

/// [`DevTools`] for a real browser.
struct Chrome(Cdp);

const CDP_TIMEOUT: Duration = Duration::from_secs(5);

fn cookie_list(result: &Value) -> Vec<Cookie> {
    let s = |c: &Value, k: &str| c.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    result
        .get("cookies")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|c| Cookie {
                    name: s(c, "name"),
                    value: s(c, "value"),
                    domain: s(c, "domain"),
                    path: s(c, "path"),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn page_ids(result: &Value) -> Vec<String> {
    result
        .get("targetInfos")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|t| t.get("type").and_then(Value::as_str) == Some("page"))
                .filter_map(|t| t.get("targetId").and_then(Value::as_str).map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

#[async_trait]
impl DevTools for Chrome {
    async fn cookies(&self) -> Result<Vec<Cookie>> {
        let r = self
            .0
            .send("Storage.getCookies", json!({}), None, CDP_TIMEOUT)
            .await?;
        Ok(cookie_list(&r))
    }

    async fn pages(&self) -> Result<usize> {
        let r = self
            .0
            .send("Target.getTargets", json!({}), None, CDP_TIMEOUT)
            .await?;
        Ok(page_ids(&r).len())
    }

    async fn remove_cookie(&self, c: &Cookie) -> Result<()> {
        let cdp = &self.0;
        let blank = cdp
            .send(
                "Target.createTarget",
                json!({"url": "about:blank"}),
                None,
                CDP_TIMEOUT,
            )
            .await?;
        let blank = blank
            .get("targetId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let targets = cdp
            .send("Target.getTargets", json!({}), None, CDP_TIMEOUT)
            .await?;
        for id in page_ids(&targets).into_iter().filter(|id| *id != blank) {
            let _ = cdp
                .send(
                    "Target.closeTarget",
                    json!({"targetId": id}),
                    None,
                    CDP_TIMEOUT,
                )
                .await;
        }
        let attached = cdp
            .send(
                "Target.attachToTarget",
                json!({"targetId": blank, "flatten": true}),
                None,
                CDP_TIMEOUT,
            )
            .await?;
        let session = attached
            .get("sessionId")
            .and_then(Value::as_str)
            .map(String::from);
        cdp.send(
            "Network.deleteCookies",
            json!({"name": c.name, "domain": c.domain, "path": c.path}),
            session.as_deref(),
            CDP_TIMEOUT,
        )
        .await?;
        let left = self.cookies().await?;
        if left
            .iter()
            .any(|x| x.name == c.name && x.domain == c.domain && x.path == c.path)
        {
            return Err(IdxError::provider(
                super::NAME,
                "the cookie was still there after deleting it",
            ));
        }
        Ok(())
    }

    async fn close(&self) {
        let _ = self
            .0
            .send("Browser.close", json!({}), None, Duration::from_secs(2))
            .await;
    }
}

// ---------------------------------------------------------------------------
// Minimal CDP client over a localhost WebSocket (no TLS)
// ---------------------------------------------------------------------------

use futures::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{client_async, WebSocketStream};

type Pending = Arc<Mutex<std::collections::HashMap<u64, oneshot::Sender<Value>>>>;
type Writer = Arc<Mutex<futures::stream::SplitSink<WebSocketStream<TcpStream>, Message>>>;

/// A tiny Chrome DevTools Protocol client: correlates command results by id.
/// Events are not read: idx subscribes to none. A background task owns the
/// read half; when the browser goes away, pending commands fail at once.
struct Cdp {
    next_id: AtomicU64,
    writer: Writer,
    pending: Pending,
}

impl Cdp {
    async fn connect(ws_url: &str) -> Result<Cdp> {
        let (host, port, _) = parse_ws_url(ws_url)?;
        let tcp = TcpStream::connect((host.as_str(), port))
            .await
            .map_err(|e| {
                IdxError::provider(super::NAME, format!("DevTools connect failed: {e}"))
            })?;
        let (ws, _) = client_async(ws_url, tcp).await.map_err(|e| {
            IdxError::provider(super::NAME, format!("DevTools handshake failed: {e}"))
        })?;
        let (sink, mut stream) = ws.split();
        let pending: Pending = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let reader_pending = pending.clone();
        tokio::spawn(async move {
            while let Some(Ok(msg)) = stream.next().await {
                let Ok(text) = msg.into_text() else { continue };
                let Ok(v) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                if let Some(id) = v.get("id").and_then(|i| i.as_u64()) {
                    if let Some(tx) = reader_pending.lock().await.remove(&id) {
                        let payload = v.get("result").cloned().unwrap_or_else(|| json!({}));
                        let _ = tx.send(payload);
                    }
                }
            }
            // The browser is gone: fail whatever still waits.
            reader_pending.lock().await.clear();
        });
        Ok(Cdp {
            next_id: AtomicU64::new(1),
            writer: Arc::new(Mutex::new(sink)),
            pending,
        })
    }

    /// Sends a command and waits for its result, bounded by `timeout`.
    async fn send(
        &self,
        method: &str,
        params: Value,
        session: Option<&str>,
        timeout: Duration,
    ) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        let mut msg = json!({"id": id, "method": method, "params": params});
        if let Some(s) = session {
            msg["sessionId"] = json!(s);
        }
        self.writer
            .lock()
            .await
            .send(Message::text(msg.to_string()))
            .await
            .map_err(|e| IdxError::provider(super::NAME, format!("DevTools write failed: {e}")))?;
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(v)) => Ok(v),
            _ => {
                self.pending.lock().await.remove(&id);
                Err(IdxError::provider(
                    super::NAME,
                    format!("{method} got no answer from the browser"),
                ))
            }
        }
    }
}

/// Splits `ws://host:port/path` into host, port, path. CDP endpoints are always
/// plain `ws://` on loopback.
fn parse_ws_url(url: &str) -> Result<(String, u16, String)> {
    let rest = url
        .strip_prefix("ws://")
        .ok_or_else(|| IdxError::provider(super::NAME, "DevTools URL was not ws://"))?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = authority.split_once(':').unwrap_or((authority, "80"));
    let port = port
        .parse()
        .map_err(|_| IdxError::provider(super::NAME, "DevTools URL had no port"))?;
    Ok((host.to_string(), port, format!("/{path}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::stockbit::auth::{Status, TokenInfo};
    use std::sync::Mutex as StdMutex;

    const COOKIE_VALUE: &str =
        include_str!("../../../tests/fixtures/stockbit/credential_storage_cookie_constructed.txt");

    fn session_cookie(value: &str, domain: &str) -> Cookie {
        Cookie {
            name: COOKIE.into(),
            value: value.into(),
            domain: domain.into(),
            path: "/".into(),
        }
    }

    /// A paste with a different refresh token each time `n` changes.
    fn other_session(n: u8) -> String {
        format!(
            r#"{{"state":{{"access":"eyJ0.eyJleHAiOjF9.a","refresh":"eyJ0.eyJleHAiOjF9.r{n}"}}}}"#
        )
    }

    /// A browser that shows `script[i]` as its cookies on the i-th poll (the
    /// last entry from then on) and `pages[i]` as its tab count.
    struct Fake {
        script: Vec<Vec<Cookie>>,
        pages: Vec<Option<usize>>,
        polls: StdMutex<usize>,
        removed: StdMutex<Vec<Cookie>>,
        closed: StdMutex<bool>,
        remove_fails: bool,
    }

    impl Fake {
        fn new(script: Vec<Vec<Cookie>>) -> Self {
            Fake {
                script,
                pages: vec![Some(1)],
                polls: StdMutex::new(0),
                removed: StdMutex::new(vec![]),
                closed: StdMutex::new(false),
                remove_fails: false,
            }
        }
        fn at<T: Clone>(v: &[T], i: usize) -> T {
            v[i.min(v.len() - 1)].clone()
        }
    }

    #[async_trait]
    impl DevTools for Fake {
        async fn cookies(&self) -> Result<Vec<Cookie>> {
            let mut n = self.polls.lock().unwrap();
            let got = Self::at(&self.script, *n);
            *n += 1;
            Ok(got)
        }
        async fn pages(&self) -> Result<usize> {
            let n = *self.polls.lock().unwrap();
            Self::at(&self.pages, n).ok_or_else(|| IdxError::Other("gone".into()))
        }
        async fn remove_cookie(&self, c: &Cookie) -> Result<()> {
            if self.remove_fails {
                return Err(IdxError::Other("no".into()));
            }
            self.removed.lock().unwrap().push(c.clone());
            Ok(())
        }
        async fn close(&self) {
            *self.closed.lock().unwrap() = true;
        }
    }

    fn times() -> WaitTimes {
        WaitTimes {
            poll: Duration::from_millis(1),
            retry: Duration::from_millis(30),
            timeout: Duration::from_secs(5),
        }
    }

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

    #[tokio::test]
    async fn a_refused_cookie_is_reported_once_then_a_new_one_is_taken() {
        let stale = session_cookie(&other_session(1), ".stockbit.com");
        let fresh = session_cookie(&other_session(2), "stockbit.com");
        let fake = Fake::new(vec![
            vec![],
            vec![stale.clone()],
            vec![stale.clone()],
            vec![stale.clone()],
            vec![stale.clone(), fresh.clone()],
        ]);
        let tried = StdMutex::new(Vec::<String>::new());
        let said = StdMutex::new(Vec::<String>::new());
        let say = |s: &str| said.lock().unwrap().push(s.to_string());
        let got = wait_for_session(
            &fake,
            times(),
            |v: String| {
                tried.lock().unwrap().push(v.clone());
                let stale = stale.value.clone();
                async move {
                    if v == stale {
                        Err(AcceptError::Refused("HTTP 401".into()))
                    } else {
                        Ok(accepted())
                    }
                }
            },
            &say,
        )
        .await
        .unwrap();
        assert!(got.status.logged_in);
        assert_eq!(
            *tried.lock().unwrap(),
            [stale.value.clone(), fresh.value.clone()]
        );
        let said = said.lock().unwrap();
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(said[0].contains("refused") && said[0].contains("HTTP 401"));
        assert!(!said[0].contains("eyJ"), "never the value");
        assert_eq!(*fake.removed.lock().unwrap(), [fresh]);
        assert!(*fake.closed.lock().unwrap());
    }

    #[tokio::test]
    async fn a_failed_check_is_retried_and_offline_ends_the_wait() {
        let c = session_cookie(COOKIE_VALUE, "stockbit.com");
        let fake = Fake::new(vec![vec![c.clone()]]);
        let calls = StdMutex::new(0);
        let got = wait_for_session(
            &fake,
            times(),
            |_v: String| {
                let n = {
                    let mut n = calls.lock().unwrap();
                    *n += 1;
                    *n
                };
                async move {
                    if n == 1 {
                        Err(AcceptError::Failed(IdxError::Network("timed out".into())))
                    } else {
                        Ok(accepted())
                    }
                }
            },
            &|_: &str| {},
        )
        .await;
        assert!(got.is_ok());
        assert_eq!(*calls.lock().unwrap(), 2);

        let fake = Fake::new(vec![vec![c]]);
        let e = wait_for_session(
            &fake,
            times(),
            |_v: String| async {
                Err::<Accepted, _>(AcceptError::Failed(IdxError::OfflineMiss("offline".into())))
            },
            &|_: &str| {},
        )
        .await
        .unwrap_err();
        assert_eq!(e.exit_code(), 5);
    }

    #[tokio::test]
    async fn closing_the_window_or_signed_out_cookies_end_or_wait() {
        // Signed out: the cookie holds no tokens and is not even tried; the
        // window is then closed (no tabs twice after one was seen).
        let signed_out = session_cookie("%7B%22state%22%3A%7B%7D%7D", "stockbit.com");
        let mut fake = Fake::new(vec![vec![signed_out]]);
        fake.pages = vec![Some(1), Some(1), Some(0), Some(0)];
        let e = wait_for_session(
            &fake,
            times(),
            |_v: String| async { Ok(accepted()) },
            &|_: &str| {},
        )
        .await
        .unwrap_err();
        assert!(e.to_string().contains("closed"), "{e}");
        // The browser process gone.
        let mut fake = Fake::new(vec![vec![]]);
        fake.pages = vec![Some(1), None];
        assert!(wait_for_session(
            &fake,
            times(),
            |_v: String| async { Ok(accepted()) },
            &|_: &str| {}
        )
        .await
        .is_err());
        // Nothing ever: the timeout.
        let fake = Fake::new(vec![vec![]]);
        let t = WaitTimes {
            timeout: Duration::from_millis(40),
            ..times()
        };
        let e = wait_for_session(
            &fake,
            t,
            |_v: String| async { Ok(accepted()) },
            &|_: &str| {},
        )
        .await
        .unwrap_err();
        assert!(e.to_string().contains("no Stockbit sign-in"), "{e}");
    }

    #[tokio::test]
    async fn a_cookie_that_will_not_go_is_a_warning_not_a_failure() {
        let c = session_cookie(COOKIE_VALUE, "stockbit.com");
        let mut fake = Fake::new(vec![vec![c]]);
        fake.remove_fails = true;
        let said = StdMutex::new(Vec::<String>::new());
        let say = |s: &str| said.lock().unwrap().push(s.to_string());
        wait_for_session(&fake, times(), |_v: String| async { Ok(accepted()) }, &say)
            .await
            .unwrap();
        assert!(said.lock().unwrap()[0].contains("could not delete the session cookie"));
        assert!(*fake.closed.lock().unwrap());
    }

    #[test]
    fn only_stockbit_cookies_count() {
        for yes in [
            "stockbit.com",
            ".stockbit.com",
            "exodus.stockbit.com",
            "STOCKBIT.COM",
        ] {
            assert!(stockbit_domain(yes), "{yes}");
        }
        for no in ["notstockbit.com", "stockbit.com.evil.test", "evil.test", ""] {
            assert!(!stockbit_domain(no), "{no}");
        }
        let foreign = session_cookie(COOKIE_VALUE, "stockbit.com.evil.test");
        let other_name = Cookie {
            name: "session".into(),
            ..session_cookie(COOKIE_VALUE, "stockbit.com")
        };
        let good = session_cookie(COOKIE_VALUE, ".stockbit.com");
        assert_eq!(candidates(vec![foreign, other_name, good.clone()]), [good]);
    }

    #[test]
    fn devtools_port_file() {
        assert_eq!(
            parse_port_file("9222\n/devtools/browser/abc-123\n"),
            Some((9222, "/devtools/browser/abc-123".into()))
        );
        assert_eq!(parse_port_file("9222\n"), None);
        assert_eq!(parse_port_file("0\n/devtools/browser/x"), None);
        assert_eq!(parse_port_file("x\n/devtools/browser/x"), None);
        assert_eq!(parse_port_file("9222\ndevtools"), None);
        assert_eq!(parse_port_file(""), None);
    }

    #[test]
    fn parses_ws_urls() {
        let (h, p, path) = parse_ws_url("ws://127.0.0.1:9541/devtools/browser/abc").unwrap();
        assert_eq!(h, "127.0.0.1");
        assert_eq!(p, 9541);
        assert_eq!(path, "/devtools/browser/abc");
        assert!(parse_ws_url("http://x").is_err());
    }

    #[test]
    fn cdp_answers_parse() {
        let cookies = cookie_list(&json!({"cookies": [
            {"name": "credentialStorage", "value": "v", "domain": ".stockbit.com", "path": "/"},
            {"name": "x"}
        ]}));
        assert_eq!(cookies.len(), 2);
        assert_eq!(cookies[0].domain, ".stockbit.com");
        assert_eq!(cookies[1].value, "");
        let pages = page_ids(&json!({"targetInfos": [
            {"type": "page", "targetId": "A"},
            {"type": "service_worker", "targetId": "B"},
            {"type": "page", "targetId": "C"}
        ]}));
        assert_eq!(pages, ["A", "C"]);
    }
}
