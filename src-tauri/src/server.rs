//! HTTP Server for receiving print jobs from the web app

use axum::{
    body::Bytes,
    extract::{Multipart, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::AppHandle;
use tower_http::cors::{Any, CorsLayer};

use crate::printer;

/// Server configuration
const HTTP_PORT: u16 = 9847;

/// Allowed origins for CORS
#[allow(dead_code)]
const ALLOWED_ORIGINS: [&str; 3] = [
    "http://localhost:3000",
    "https://anymobileus.com",
    "https://www.anymobileus.com",
];

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

/// Start the HTTP server
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

    // Start HTTP server
    let addr = format!("127.0.0.1:{}", HTTP_PORT);
    tracing::info!("Starting HTTP server on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

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
