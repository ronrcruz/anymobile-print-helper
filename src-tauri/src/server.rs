//! HTTP/HTTPS Server for receiving print jobs from the web app

use axum::{
    body::Bytes,
    extract::{Multipart, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use axum_server::tls_rustls::RustlsConfig;
use rcgen::{CertifiedKey, generate_simple_self_signed};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::path::PathBuf;
use std::fs;
use tauri::AppHandle;
use tower_http::cors::{Any, CorsLayer};

use crate::printer;

/// Server configuration
pub const HTTPS_PORT: u16 = 9847;
pub const HTTP_PORT: u16 = 9848;

/// Server state
struct ServerState {
    app_handle: AppHandle,
}

/// Response for /ping endpoint
#[derive(Serialize)]
struct PingResponse {
    app: &'static str,
    version: String,
    printers: Vec<PrinterInfo>,
}

/// Printer information
#[derive(Serialize, Clone)]
pub struct PrinterInfo {
    pub name: String,
    #[serde(rename = "isDefault")]
    pub is_default: bool,
    pub status: String,
}

/// Response for /print endpoint
#[derive(Serialize)]
struct PrintResponse {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "jobId")]
    job_id: Option<String>,
}

/// Print request options
#[derive(Deserialize, Default)]
struct PrintOptions {
    printer: Option<String>,
    copies: Option<u32>,
}

/// Get the path to store certificates
fn get_cert_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("anymobile-print-helper")
        .join("certs")
}

/// Generate or load a self-signed certificate for localhost
fn get_or_create_certificate() -> Result<(Vec<u8>, Vec<u8>), Box<dyn std::error::Error + Send + Sync>> {
    let cert_dir = get_cert_dir();
    let cert_path = cert_dir.join("localhost.crt");
    let key_path = cert_dir.join("localhost.key");

    // Check if certificate already exists and is valid
    if cert_path.exists() && key_path.exists() {
        tracing::info!("Loading existing certificate from {:?}", cert_dir);
        match (fs::read(&cert_path), fs::read(&key_path)) {
            (Ok(cert_pem), Ok(key_pem)) if !cert_pem.is_empty() && !key_pem.is_empty() => {
                return Ok((cert_pem, key_pem));
            }
            _ => {
                tracing::warn!("Existing certificate is invalid, regenerating...");
                let _ = fs::remove_file(&cert_path);
                let _ = fs::remove_file(&key_path);
            }
        }
    }

    // Generate new self-signed certificate
    tracing::info!("Generating new self-signed certificate");
    let subject_alt_names = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
    ];

    let CertifiedKey { cert, key_pair } = generate_simple_self_signed(subject_alt_names)
        .map_err(|e| format!("Failed to generate certificate: {}", e))?;

    let cert_pem = cert.pem().into_bytes();
    let key_pem = key_pair.serialize_pem().into_bytes();

    // Save certificate for future use
    if let Err(e) = fs::create_dir_all(&cert_dir) {
        tracing::warn!("Could not create cert directory: {}", e);
    } else {
        if let Err(e) = fs::write(&cert_path, &cert_pem) {
            tracing::warn!("Could not save certificate: {}", e);
        }
        if let Err(e) = fs::write(&key_path, &key_pem) {
            tracing::warn!("Could not save key: {}", e);
        } else {
            tracing::info!("Saved certificate to {:?}", cert_dir);
        }
    }

    Ok((cert_pem, key_pem))
}

/// Start both HTTPS and HTTP servers
pub async fn start_server(app_handle: AppHandle) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let state = Arc::new(ServerState { app_handle });

    // Build CORS layer - permissive for local desktop app
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Build router
    let app: Router = Router::new()
        .route("/ping", get(handle_ping))
        .route("/printers", get(handle_printers))
        .route("/print", post(handle_print))
        .layer(cors)
        .with_state(state);

    // Get or create SSL certificate
    let (cert_pem, key_pem) = get_or_create_certificate()?;

    // Configure TLS
    let tls_config = RustlsConfig::from_pem(cert_pem, key_pem).await?;

    // Clone app for HTTP server
    let http_app = app.clone();

    // Start HTTP fallback server on secondary port (for Windows/Chrome/Firefox)
    tokio::spawn(async move {
        let http_addr = format!("127.0.0.1:{}", HTTP_PORT);
        if let Ok(listener) = tokio::net::TcpListener::bind(&http_addr).await {
            tracing::info!("HTTP server listening on {}", http_addr);
            let _ = axum::serve(listener, http_app).await;
        }
    });

    // Start HTTPS server on primary port (for Safari)
    let https_addr = format!("127.0.0.1:{}", HTTPS_PORT);
    tracing::info!("Starting HTTPS server on {}", https_addr);

    axum_server::bind_rustls(https_addr.parse()?, tls_config)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

/// Handle /ping - health check and version info
async fn handle_ping(State(state): State<Arc<ServerState>>) -> Json<PingResponse> {
    let printers = printer::list_printers().unwrap_or_default();
    let version = state
        .app_handle
        .package_info()
        .version
        .to_string();

    Json(PingResponse {
        app: "anymobile-print-helper",
        version,
        printers,
    })
}

/// Handle /printers - list available printers
async fn handle_printers() -> Json<serde_json::Value> {
    let printers = printer::list_printers().unwrap_or_default();
    Json(serde_json::json!({ "printers": printers }))
}

/// Handle /print - receive PDF and print it
async fn handle_print(
    State(_state): State<Arc<ServerState>>,
    mut multipart: Multipart,
) -> Result<Json<PrintResponse>, (StatusCode, Json<PrintResponse>)> {
    let mut pdf_data: Option<Bytes> = None;
    let mut options = PrintOptions::default();

    // Parse multipart form data
    while let Some(field) = multipart.next_field().await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(PrintResponse {
                success: false,
                error: Some(format!("Failed to parse form data: {}", e)),
                job_id: None,
            }),
        )
    })? {
        let name = field.name().unwrap_or_default().to_string();

        match name.as_str() {
            "pdf" => {
                pdf_data = Some(field.bytes().await.map_err(|e| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(PrintResponse {
                            success: false,
                            error: Some(format!("Failed to read PDF data: {}", e)),
                            job_id: None,
                        }),
                    )
                })?);
            }
            "printer" => {
                if let Ok(text) = field.text().await {
                    options.printer = Some(text);
                }
            }
            "copies" => {
                if let Ok(text) = field.text().await {
                    options.copies = text.parse().ok();
                }
            }
            _ => {}
        }
    }

    // Ensure we have PDF data
    let pdf_data = pdf_data.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(PrintResponse {
                success: false,
                error: Some("No PDF data provided".to_string()),
                job_id: None,
            }),
        )
    })?;

    // Save PDF to temp file and print
    match printer::print_pdf(&pdf_data, options.printer.as_deref(), options.copies.unwrap_or(1)).await {
        Ok(job_id) => Ok(Json(PrintResponse {
            success: true,
            error: None,
            job_id: Some(job_id),
        })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(PrintResponse {
                success: false,
                error: Some(e.to_string()),
                job_id: None,
            }),
        )),
    }
}
