#![forbid(unsafe_code)]
#![deny(missing_docs, missing_debug_implementations)]

//! # `echo-rs` - a simple echo server

// Standard Library Imports
use std::{collections::HashMap, env, fmt::Debug, net::SocketAddr, path::PathBuf};

// Third Party Imports
use axum::{
    body::Bytes,
    extract::{ConnectInfo, Json, Path, Query},
    http::{HeaderMap, Method},
    middleware, routing, Router,
};
use axum_server::tls_rustls::RustlsConfig;

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
    #[arg(long = "port", env = "ECHO_PORT", default_value_t = 8080)]
    pub port: usize,
    #[arg(long = "metrics", env = "ECHO_METRICS", default_value_t = true)]
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
}

#[tracing::instrument(ret, skip_all, parent = None)]
async fn serialize_request(
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    method: Method,
    path: Option<Path<String>>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Json<Echo> {
    let mut path = path.map(|value| value.0).unwrap_or_default();

    if !path.starts_with('/') {
        // path extractor sometimes omits leading slash
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

    Json(Echo {
        client,
        method,
        path,
        headers,
        params,
        body,
    })
}

#[tracing::instrument]
async fn echo_router() -> anyhow::Result<Router> {
    Ok(Router::new()
        .route(
            "/",
            routing::get(serialize_request)
                .put(serialize_request)
                .head(serialize_request)
                .post(serialize_request)
                .patch(serialize_request)
                .trace(serialize_request)
                .options(serialize_request),
        )
        .route(
            "/*key",
            routing::get(serialize_request)
                .put(serialize_request)
                .head(serialize_request)
                .post(serialize_request)
                .patch(serialize_request)
                .trace(serialize_request)
                .options(serialize_request),
        )
        .fallback(serialize_request)
        .route_layer(middleware::from_fn(metrics::track_metrics)))
}

#[tracing::instrument(skip_all)]
async fn serve_app(
    host: &str,
    port: usize,
    tls_key: Option<&PathBuf>,
    tls_cert: Option<&PathBuf>,
) -> anyhow::Result<()> {
    let app = echo_router().await?;

    const LOG_LINE: &str = "`echo-rs` server listening at";

    let (mut proto, addr) = (
        "http".to_string(),
        format!("{host}:{port}").parse::<SocketAddr>()?,
    );

    match (tls_key, tls_cert) {
        (Some(key), Some(cert)) => {
            proto.push('s');

            // configure certificate and private key used by https
            let tls_config = RustlsConfig::from_pem_file(cert, key).await.unwrap();

            tracing::info!("{LOG_LINE}: {proto}://{addr}");

            axum_server::bind_rustls(addr, tls_config)
                .serve(app.into_make_service_with_connect_info::<SocketAddr>())
                .await
                .unwrap();
        }
        _ => {
            tracing::info!("{LOG_LINE}: {proto}://{addr}");

            axum::Server::bind(&addr)
                .serve(app.into_make_service_with_connect_info::<SocketAddr>())
                .await?;
        }
    };

    Ok(())
}

#[tracing::instrument(skip_all)]
async fn serve_metrics(
    host: &str,
    port: usize,
    tls_key: Option<&PathBuf>,
    tls_cert: Option<&PathBuf>,
) -> anyhow::Result<()> {
    let app = metrics::router();

    const LOG_LINE: &str = "Serving Prometheus metrics at";

    let (mut proto, addr) = (
        "http".to_string(),
        format!("{host}:{port}").parse::<SocketAddr>()?,
    );

    match (tls_key, tls_cert) {
        (Some(key), Some(cert)) => {
            proto.push('s');

            // configure certificate and private key used by https
            let tls_config = RustlsConfig::from_pem_file(cert, key).await.unwrap();

            tracing::info!("{LOG_LINE}: {proto}://{addr}");

            axum_server::bind_rustls(addr, tls_config)
                .serve(app.into_make_service_with_connect_info::<SocketAddr>())
                .await
                .unwrap();
        }
        _ => {
            tracing::info!("{LOG_LINE}: {proto}://{addr}");

            axum::Server::bind(&addr)
                .serve(app.into_make_service_with_connect_info::<SocketAddr>())
                .await?;
        }
    };

    Ok(())
}

#[tracing::instrument]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = <Args as clap::Parser>::parse();

    let mut log_conf = env::var("RUST_LOG").unwrap_or_default();

    if !log_conf.to_ascii_lowercase().contains("echo_rs") {
        if !log_conf.is_empty() {
            log_conf.insert(log_conf.len(), ',');
        }

        log_conf.extend(format!("echo_rs={}", args.log_level.as_str()).chars());
    }

    env::set_var("RUST_LOG", log_conf);

    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("RUST_LOG")
                .unwrap_or(tracing_subscriber::EnvFilter::from_default_env()),
        )
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    if !args.metrics {
        serve_app(
            &args.host,
            args.port,
            args.tls_key.as_ref(),
            args.tls_cert.as_ref(),
        )
        .await
    } else {
        let (echo_server, metrics_server) = tokio::join!(
            serve_app(
                &args.host,
                args.port,
                args.tls_key.as_ref(),
                args.tls_cert.as_ref()
            ),
            if !args.metrics_use_tls {
                serve_metrics(&args.host, args.metrics_port, None, None)
            } else {
                serve_metrics(
                    &args.host,
                    args.metrics_port,
                    args.tls_key.as_ref(),
                    args.tls_cert.as_ref(),
                )
            }
        );
        let (_, _) = (echo_server?, metrics_server?);

        Ok(())
    }
}
