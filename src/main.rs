#![forbid(unsafe_code)]
#![deny(missing_docs, missing_debug_implementations)]

//! # `echo-rs` - a simple echo server

// Standard Library Imports
use std::{collections::HashMap, env, fmt::Debug, net::SocketAddr};

// Third Party Imports
use axum::{
    body::Bytes,
    extract::{Json, Path, Query},
    http::{HeaderMap, Method},
    middleware, routing, Router,
};

pub(crate) mod metrics;

#[derive(Debug, serde::Serialize, Clone)]
struct Echo {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    params: HashMap<String, String>,
    body: serde_json::Value,
}

#[derive(Debug, clap::Parser)]
#[command(author, version, about)]
struct Args {
    #[arg(short = 'p', long = "port", env = "ECHO_PORT", default_value_t = 8080)]
    pub port: usize,
    #[arg(
        short = 'm',
        long = "metrics",
        env = "ECHO_METRICS",
        default_value_t = true
    )]
    pub metrics: core::primitive::bool,
    #[arg(
        long = "metrics-port",
        env = "ECHO_METRICS_PORT",
        default_value_t = 9090
    )]
    pub metrics_port: usize,
    #[arg(
        short = 'l',
        long = "log-level",
        env = "ECHO_LOG_LEVEL",
        default_value_t = tracing::Level::INFO,
    )]
    pub log_level: tracing::Level,
}

#[tracing::instrument]
async fn serialize_request(
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

    let method = method.to_string();

    tracing::info!("{} {}", &method, &path);

    Json(Echo {
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

#[tracing::instrument]
async fn serve_app(port: usize) -> anyhow::Result<()> {
    let app = echo_router().await?;

    let addr: SocketAddr = format!("[::]:{port}").parse()?;

    tracing::info!("`echo-rs` server listening at: http://{addr}");

    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

#[tracing::instrument]
async fn serve_metrics(port: usize) -> anyhow::Result<()> {
    let app = metrics::router();

    let addr: SocketAddr = format!("[::]:{port}").parse()?;

    tracing::info!("Serving Prometheus metrics at: http://{addr}");

    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();

    Ok(())
}

#[tokio::main]
#[tracing::instrument]
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
        serve_app(args.port).await
    } else {
        let (echo_server, metrics_server) =
            tokio::join!(serve_app(args.port), serve_metrics(args.metrics_port));
        let (_, _) = (echo_server?, metrics_server?);

        Ok(())
    }
}
