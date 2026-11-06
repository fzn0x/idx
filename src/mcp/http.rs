//! `idx mcp --http ADDR`: the same server over MCP streamable HTTP, for
//! clients that only connect to a URL (ChatGPT, Claude.ai connectors,
//! Perplexity, Devin, Manus, hosted agents) and for sharing one idx cache
//! between several local agents.
//!
//! Endpoint: `http://ADDR/mcp`. `GET /healthz` answers `ok`. A bearer token
//! (`--token` / `IDX_MCP_TOKEN`) is required when binding anything other
//! than a loopback address, and recommended behind a tunnel. The Host
//! header allow-list defaults to loopback names; pass `--allowed-host` for a
//! tunnel hostname or `*` to disable the check.

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};

use crate::error::{IdxError, Result};
use crate::http::Fetcher;
use crate::providers::Provider;

use super::{IdxServer, StockbitConfig};

#[derive(Debug, Clone)]
pub struct HttpOptions {
    /// `host:port`; port 0 picks a free one.
    pub bind: String,
    pub token: Option<String>,
    /// Extra Host header values to accept; `*` accepts any.
    pub allowed_hosts: Vec<String>,
}

async fn auth(State(token): State<Arc<String>>, req: Request, next: Next) -> Response {
    if req.uri().path() == "/healthz" {
        return next.run(req).await;
    }
    let ok = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|t| t.trim() == token.as_str());
    if ok {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            "idx mcp: missing or wrong bearer token",
        )
            .into_response()
    }
}

/// Builds the router; separated from `serve_http` so tests can drive it.
pub fn router(
    provider: Arc<dyn Provider>,
    fetcher: Arc<Fetcher>,
    ownership_db: PathBuf,
    stockbit: StockbitConfig,
    token: Option<String>,
    allowed_hosts: Vec<String>,
) -> Router {
    let mut config = StreamableHttpServerConfig::default();
    if allowed_hosts.iter().any(|h| h == "*") {
        config.allowed_hosts.clear(); // empty = accept any Host
    } else {
        config.allowed_hosts.extend(allowed_hosts);
    }
    // One session store for every HTTP session's server.
    let stockbit = stockbit.resolved();
    let service = StreamableHttpService::new(
        move || {
            Ok(IdxServer::new(
                provider.clone(),
                fetcher.clone(),
                ownership_db.clone(),
                stockbit.clone(),
            ))
        },
        Arc::new(LocalSessionManager::default()),
        config,
    );
    let app = Router::new()
        .nest_service("/mcp", service)
        .route("/healthz", get(|| async { "ok" }));
    match token {
        Some(t) => app.layer(middleware::from_fn_with_state(Arc::new(t), auth)),
        None => app,
    }
}

/// Serves until Ctrl-C.
pub async fn serve_http(
    provider: Arc<dyn Provider>,
    fetcher: Arc<Fetcher>,
    ownership_db: PathBuf,
    stockbit: StockbitConfig,
    opts: HttpOptions,
) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(&opts.bind)
        .await
        .map_err(|e| IdxError::Config(format!("cannot bind {}: {e}", opts.bind)))?;
    let addr = listener
        .local_addr()
        .map_err(|e| IdxError::Other(format!("local_addr: {e}")))?;
    if !addr.ip().is_loopback() && opts.token.is_none() {
        return Err(IdxError::Config(format!(
            "{addr} is reachable from other machines; set --token (or IDX_MCP_TOKEN) or bind 127.0.0.1"
        )));
    }
    let mut hosts = opts.allowed_hosts.clone();
    if !addr.ip().is_loopback() {
        hosts.push(addr.to_string());
    }
    let app = router(
        provider,
        fetcher,
        ownership_db,
        stockbit,
        opts.token.clone(),
        hosts,
    );
    eprintln!(
        "idx mcp: streamable HTTP listening on http://{addr}/mcp{}{}",
        if opts.token.is_some() {
            " (bearer token required)"
        } else {
            ""
        },
        if opts.allowed_hosts.iter().any(|h| h == "*") {
            " · any Host accepted"
        } else {
            ""
        }
    );
    tracing::info!(%addr, "mcp http server started");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|e| IdxError::Other(format!("mcp http: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{History, Interval, Period, Quote, Symbol};
    use crate::providers::Sourced;
    use async_trait::async_trait;
    use axum::body::Body;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    struct Fake;
    #[async_trait]
    impl Provider for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }
        async fn quotes(&self, s: &[Symbol]) -> Result<Sourced<Vec<Quote>>> {
            let mut q = Quote::empty(s[0].clone(), "fake");
            q.price = Some(1.0);
            Ok(Sourced::fresh(vec![q], "fake"))
        }
        async fn history(&self, _: &Symbol, _: Period, _: Interval) -> Result<Sourced<History>> {
            Err(IdxError::OfflineMiss("nothing cached".into()))
        }
    }

    fn app(token: Option<&str>) -> Router {
        let dir = std::env::temp_dir().join(format!("idx-mcp-http-{}", std::process::id()));
        let fetcher = Arc::new(Fetcher::new(
            Arc::new(crate::http::FixtureBackend::new()),
            crate::cache::Cache::new(dir.join("cache")),
            crate::http::CacheMode::Offline,
        ));
        router(
            Arc::new(Fake),
            fetcher,
            dir.join("own.sqlite"),
            StockbitConfig::default(),
            token.map(str::to_string),
            vec![],
        )
    }

    fn initialize() -> String {
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}
        })
        .to_string()
    }

    fn post(body: String, bearer: Option<&str>) -> Request {
        let mut b = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("host", "127.0.0.1")
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream");
        if let Some(t) = bearer {
            b = b.header("authorization", format!("Bearer {t}"));
        }
        b.body(Body::from(body)).unwrap()
    }

    #[tokio::test]
    async fn initialize_over_http_and_token_gate() {
        let res = app(None).oneshot(post(initialize(), None)).await.unwrap();
        assert_eq!(
            res.status(),
            StatusCode::OK,
            "open server answers initialize"
        );
        let body = res.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("\"serverInfo\""), "{text}");
        assert!(text.contains("\"name\":\"idx\""), "{text}");

        let res = app(Some("s3cret"))
            .oneshot(post(initialize(), None))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let res = app(Some("s3cret"))
            .oneshot(post(initialize(), Some("s3cret")))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let res = app(Some("s3cret"))
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .header("host", "127.0.0.1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "health check needs no token");
    }
}
