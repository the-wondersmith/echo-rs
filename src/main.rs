#![forbid(unsafe_code)]
#![deny(missing_docs, missing_debug_implementations)]

//! # `echo-rs` - a simple echo server

// Standard Library Imports
use std::{collections::HashMap, env, fmt::Debug, net::SocketAddr, path::PathBuf, sync::Arc};

// Third Party Imports
use axum::{
    body::Bytes,
    extract::{ConnectInfo, Json, Path, Query, State},
    http::{HeaderMap, Method},
    middleware, routing, Router,
};
use axum_server::tls_rustls::RustlsConfig;
use regex_lite::Regex;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::util::SubscriberInitExt;

pub(crate) mod metrics;

#[derive(Clone, Debug, serde::Serialize)]
struct Echo {
    client: String,
    method: String,
    path: String,
    headers: HashMap<String, String>,
    params: HashMap<String, String>,
    body: serde_json::Value,
}

#[derive(Clone, Debug, clap::Parser)]
#[command(author, version, about)]
struct Args {
    #[arg(long = "host", env = "ECHO_HOST", default_value = "[::]")]
    pub host: String,

    #[arg(long = "port", env = "ECHO_PORT", default_value_t = 8000)]
    pub port: usize,

    #[arg(long = "metrics", env = "ECHO_METRICS", default_value_t = false)]
    pub metrics: core::primitive::bool,

    #[arg(
        long = "metrics-port",
        env = "ECHO_METRICS_PORT",
        default_value_t = 9090
    )]
    pub metrics_port: usize,

    #[arg(
        long = "log-level",
        env = "ECHO_LOG_LEVEL",
        default_value_t = tracing::Level::INFO,
    )]
    pub log_level: tracing::Level,

    #[arg(long = "tls-key", env = "ECHO_TLS_KEY")]
    pub tls_key: Option<PathBuf>,

    #[arg(long = "tls-cert", env = "ECHO_TLS_CERT")]
    pub tls_cert: Option<PathBuf>,

    #[arg(
        long = "metrics-use-tls",
        env = "ECHO_METRICS_USE_TLS",
        default_value_t = false
    )]
    pub metrics_use_tls: bool,

    #[arg(
        long = "skip-logging-for",
        env = "ECHO_SKIP_LOGGING_FOR",
        default_value = "",
        long_help = "Comma or semi-colon separated list of URL patterns that should not be logged.\n\nExample:\n  echo-rs ... --skip-logging-for='some/endpoint; another/endpoint\\?with=some-param'"
    )]
    pub unlogged: String,
}

#[tracing::instrument(skip_all, parent = None)]
/// Parse user-supplied patterns for URLs that should not be logged
fn parse_unlogged_patterns(value: &str) -> Vec<Regex> {
    let mut patterns: Vec<Regex> = Vec::new();

    if !value.is_empty() {
        patterns.extend(Regex::new("[,;] ?").unwrap().split(value).flat_map(
            |pat| match Regex::new(pat) {
                Ok(pattern) => Some(pattern),
                Err(_) => {
                    tracing::warn!("Declining to add bad filter pattern: {pat}");
                    None
                }
            },
        ));
    }

    patterns
}

#[tracing::instrument(skip_all, parent = None)]
async fn serialize_request(
    State(url_filters): State<Arc<Vec<Regex>>>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    method: Method,
    path: Option<Path<String>>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Json<Echo> {
    let mut path = path.map(|value| value.0).unwrap_or_default();

    if !path.starts_with('/') {
        // path extractor sometimes omits leading slashes
        path.insert(0, '/');
    }

    let headers = headers
        .into_iter()
        .filter(|(name, _)| name.is_some())
        .map(|(name, value)| {
            (
                name.unwrap().as_str().to_owned(),
                value.to_str().unwrap_or("<non-ascii string>").to_owned(),
            )
        })
        .collect::<HashMap<String, String>>();

    let body = if body.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice::<serde_json::Value>(&body).unwrap_or_else(|_| {
            serde_json::Value::Array(
                body.iter()
                    .map(|value| serde_json::Value::Number((*value).into()))
                    .collect::<Vec<serde_json::Value>>(),
            )
        })
    };

    let (client, method) = (client.to_string(), method.to_string());

    let request = Echo {
        client,
        method,
        path,
        headers,
        params,
        body,
    };

    if !url_filters
        .iter()
        .any(|pattern| pattern.is_match(&request.path))
    {
        tracing::info!("{request:?}");
    }

    Json(request)
}

#[tracing::instrument]
async fn echo_router(url_filters: Arc<Vec<Regex>>) -> anyhow::Result<Router> {
    Ok(Router::new()
        .route("/{*wildcard}", routing::any(serialize_request))
        .fallback(serialize_request)
        .with_state(url_filters)
        .layer(middleware::from_fn(metrics::track_metrics)))
}

#[tracing::instrument(skip_all)]
async fn serve_app(
    host: impl std::fmt::Display,
    port: usize,
    token: CancellationToken,
    tls_key: Option<impl AsRef<std::path::Path>>,
    tls_cert: Option<impl AsRef<std::path::Path>>,
    url_filters: Vec<Regex>,
) -> anyhow::Result<()> {
    let app = echo_router(Arc::new(url_filters))
        .await?
        .into_make_service_with_connect_info::<SocketAddr>();

    const LOG_LINE: &str = "`echo-rs` server listening at";

    let (mut protocol, address) = (
        "http".to_string(),
        format!("{host}:{port}").parse::<SocketAddr>()?,
    );

    match (tls_key, tls_cert) {
        (Some(key), Some(cert)) => {
            protocol.push('s');

            // configure certificate and private key used by https
            let tls_config = RustlsConfig::from_pem_file(cert.as_ref(), key.as_ref()).await?;

            tracing::info!("{LOG_LINE}: {protocol}://{address}");

            let server = axum_server::bind_rustls(address, tls_config).serve(app);

            tokio::select! {
                _ = server => {},
                _ = token.cancelled() => {},
            }
        }
        _ => {
            tracing::info!("{LOG_LINE}: {protocol}://{address}");

            let listener = tokio::net::TcpListener::bind(address).await?;

            axum::serve(listener, app)
                .with_graceful_shutdown(async move { token.cancelled().await })
                .await?;
        }
    };

    tracing::info!("`echo-rs` server stopped");

    Ok(())
}

#[tracing::instrument(skip_all)]
async fn serve_metrics(
    host: &str,
    port: usize,
    token: CancellationToken,
    tls_key: Option<&PathBuf>,
    tls_cert: Option<&PathBuf>,
) -> anyhow::Result<()> {
    let app = metrics::router();

    const LOG_LINE: &str = "Serving Prometheus metrics at";

    let (mut protocol, address) = (
        "http".to_string(),
        format!("{host}:{port}").parse::<SocketAddr>()?,
    );

    match (tls_key, tls_cert) {
        (Some(key), Some(cert)) => {
            protocol.push('s');

            // configure certificate and private key used by https
            let tls_config = RustlsConfig::from_pem_file(cert, key).await?;

            tracing::info!("{LOG_LINE}: {protocol}://{address}");

            let server = axum_server::bind_rustls(address, tls_config)
                .serve(app.into_make_service_with_connect_info::<SocketAddr>());

            tokio::select! {
                _ = server => {},
                _ = token.cancelled() => {},
            }
        }
        _ => {
            tracing::info!("{LOG_LINE}: {protocol}://{address}");

            let listener = tokio::net::TcpListener::bind(address).await?;

            axum::serve(listener, app)
                .with_graceful_shutdown(async move { token.cancelled().await })
                .await?;
        }
    };

    tracing::info!("Metrics server stopped");

    Ok(())
}

/// Listen for any shutdown signal
async fn listen_for_shutdown(token: CancellationToken) {
    use tokio::signal::unix::{signal as install_signal, SignalKind};

    #[cfg(not(any(unix, target_os = "linux")))]
    compile_error!("`echo-rs` does not support non-unix-like hosts!");

    let (mut sig_quit, mut sig_alarm, mut sig_hangup, mut sig_interrupt, mut sig_terminate) = (
        install_signal(SignalKind::quit()).expect("failed to install SIGQUIT handler"),
        install_signal(SignalKind::alarm()).expect("failed to install SIGALRM handler"),
        install_signal(SignalKind::hangup()).expect("failed to install SIGHUP handler"),
        install_signal(SignalKind::interrupt()).expect("failed to install SIGINT handler"),
        install_signal(SignalKind::terminate()).expect("failed to install SIGTERM handler"),
    );

    tokio::select! {
        _ = sig_quit.recv() =>         {
            tracing::info!("received SIGQUIT");
            token.cancel();
        },
        _ = sig_alarm.recv() =>        {
            tracing::info!("received SIGALRM");
            token.cancel();
        },
        _ = sig_hangup.recv() =>       {
            tracing::info!("received SIGHUP");
            token.cancel();
        },
        _ = sig_interrupt.recv() =>    {
            tracing::info!("received SIGINT");
            token.cancel();
        },
        _ = sig_terminate.recv() =>    {
            tracing::info!("received SIGTERM");
            token.cancel();
        },
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("received ^C");
            token.cancel();
        },
        _ = token.cancelled()
         => {
            tracing::info!("CancellationToken cancelled");
        }
    }
}

#[tracing::instrument]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = <Args as clap::Parser>::parse();

    if !args.metrics_use_tls {
        args.tls_key = None;
        args.tls_cert = None;
    } else if args.metrics_use_tls && (args.tls_key.is_none() || args.tls_cert.is_none()) {
        anyhow::bail!("--metrics-use-tls requires --tls-key and --tls-cert to be set");
    }

    let mut log_conf = env::var("RUST_LOG")
        .as_deref()
        .unwrap_or(args.log_level.as_str())
        .to_string();

    if log_conf.is_empty() {
        log_conf.push_str(args.log_level.as_str());
    }

    if !log_conf.to_ascii_lowercase().contains("echo_rs") {
        log_conf.push_str(&format!(",echo_rs={}", &args.log_level).to_ascii_lowercase());
    }

    tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(tracing_subscriber::EnvFilter::from(&log_conf))
        .finish()
        .init();

    let token = CancellationToken::new();
    let url_filters = parse_unlogged_patterns(&args.unlogged);

    let echo_server = serve_app(
        args.host.clone(),
        args.port,
        token.clone(),
        args.tls_key.clone(),
        args.tls_cert.clone(),
        url_filters,
    );

    let metrics_server = {
        let token = token.clone();

        async move {
            if !args.metrics {
                std::future::pending::<()>().await;

                Ok(())
            } else {
                serve_metrics(
                    &args.host,
                    args.metrics_port,
                    token,
                    args.tls_key.as_ref(),
                    args.tls_cert.as_ref(),
                )
                .await
            }
        }
    };

    tokio::select! {
        result = echo_server => result,
        result = metrics_server => result,
        _ = listen_for_shutdown(token) => Ok(()),
    }
}
