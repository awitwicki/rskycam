mod auth;
mod camera;
mod capture;
mod overlay;
mod processing;
mod sensors;
mod settings;
mod system;
mod web;

use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "rskycam=info".into()),
        )
        .init();

    let data_dir = std::path::PathBuf::from(
        std::env::var("RSKYCAM_DATA").unwrap_or_else(|_| "/var/lib/rskycam".into()),
    );
    let port: u16 = std::env::var("RSKYCAM_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    std::fs::create_dir_all(&data_dir)?;

    let store = Arc::new(settings::SettingsStore::new(&data_dir));
    let cfg = store.load_or_create(&auth::hash_password(auth::DEFAULT_PASSWORD)?)?;
    let cfg = Arc::new(tokio::sync::RwLock::new(cfg));
    let key = web::load_or_create_secret(&data_dir)?;

    let processing = processing::spawn_processing(
        cfg.clone(),
        data_dir.clone(),
        processing::ProcessingConfig {
            ffmpeg: "ffmpeg".into(),
            dawn_check: std::time::Duration::from_secs(60),
        },
    );
    processing::retention::spawn_retention(cfg.clone(), data_dir.clone());
    let channels = capture::spawn_capture(
        cfg.clone(),
        data_dir.clone(),
        Some(processing.frames.clone()),
    );
    let state = web::AppState {
        cfg,
        store,
        latest: channels.latest.clone(),
        capture_status: channels.status.clone(),
        camera_caps: channels.camera_caps.clone(),
        key,
        data_dir,
        processing: processing.clone(),
    };

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("rskycam listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, web::router(state)).await?;
    Ok(())
}
