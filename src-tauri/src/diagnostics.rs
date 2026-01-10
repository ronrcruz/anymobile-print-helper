//! Diagnostic utilities for troubleshooting connection issues

use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};
use std::fmt::Write as FmtWrite;
use once_cell::sync::Lazy;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

use crate::cert_manager;
use crate::server::{HTTPS_PORT, HTTP_PORT, PrinterInfo};
use crate::printer;

/// Maximum number of log entries to keep in memory
const MAX_LOG_ENTRIES: usize = 500;

/// In-memory log buffer (circular buffer)
static LOG_BUFFER: Lazy<RwLock<Vec<LogEntry>>> = Lazy::new(|| RwLock::new(Vec::with_capacity(MAX_LOG_ENTRIES)));
static LOG_COUNTER: Lazy<std::sync::atomic::AtomicU64> = Lazy::new(|| std::sync::atomic::AtomicU64::new(0));

/// App start time for uptime calculation
static APP_START_TIME: Lazy<u64> = Lazy::new(|| {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
});

/// Overall status enum
#[derive(Serialize, Clone, Debug)]
pub enum OverallStatus {
    Ready,
    Warning,
    Error,
}

/// Full diagnostic status for the UI
#[derive(Serialize, Clone, Debug)]
pub struct DiagnosticStatus {
    pub https_running: bool,
    pub http_running: bool,
    pub cert_exists: bool,
    pub cert_valid: bool,
    pub cert_trusted: bool,
    pub cert_path: String,
    pub version: String,
    pub uptime_seconds: u64,
    pub platform: String,
    pub overall_status: OverallStatus,
}

/// Certificate information
#[derive(Serialize, Clone, Debug)]
pub struct CertificateInfo {
    pub exists: bool,
    pub path: String,
    pub file_size_bytes: Option<u64>,
    pub created: Option<String>,
    pub modified: Option<String>,
    pub is_trusted: bool,
}

/// Connection test result
#[derive(Serialize, Clone, Debug)]
pub struct ConnectionTestResult {
    pub success: bool,
    pub https_ok: bool,
    pub http_ok: bool,
    pub https_latency_ms: Option<u64>,
    pub http_latency_ms: Option<u64>,
    pub localhost_resolves: bool,
    pub loopback_accessible: bool,
    pub message: String,
}

/// Log entry for UI display
#[derive(Serialize, Clone, Debug)]
pub struct LogEntry {
    pub id: u64,
    pub timestamp: String,
    pub level: String,
    pub source: String,
    pub message: String,
}

/// Add a log entry to the buffer
pub fn add_log_entry(level: &str, source: &str, message: &str) {
    let entry = LogEntry {
        id: LOG_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
        timestamp: chrono::Local::now().format("%H:%M:%S%.3f").to_string(),
        level: level.to_string(),
        source: source.to_string(),
        message: message.to_string(),
    };

    if let Ok(mut buffer) = LOG_BUFFER.write() {
        if buffer.len() >= MAX_LOG_ENTRIES {
            buffer.remove(0);
        }
        buffer.push(entry);
    }
}

/// Get recent logs from buffer
pub fn get_recent_logs(count: Option<usize>, level_filter: Option<&str>) -> Vec<LogEntry> {
    let buffer = match LOG_BUFFER.read() {
        Ok(b) => b,
        Err(_) => return vec![],
    };

    let filtered: Vec<LogEntry> = buffer
        .iter()
        .filter(|entry| {
            level_filter.map_or(true, |filter| {
                entry.level.eq_ignore_ascii_case(filter)
            })
        })
        .cloned()
        .collect();

    let limit = count.unwrap_or(100).min(filtered.len());
    filtered.iter().rev().take(limit).cloned().collect()
}

/// Clear log buffer
pub fn clear_logs() {
    if let Ok(mut buffer) = LOG_BUFFER.write() {
        buffer.clear();
    }
}

// ============================================================================
// Custom Tracing Layer - forwards logs to LOG_BUFFER
// ============================================================================

/// Custom tracing layer that captures log events and adds them to the in-memory buffer
pub struct LogBufferLayer;

/// Visitor to extract the message field from tracing events
struct MessageVisitor {
    message: String,
}

impl MessageVisitor {
    fn new() -> Self {
        Self { message: String::new() }
    }
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(&mut self.message, "{:?}", value);
            // Remove surrounding quotes if present
            if self.message.starts_with('"') && self.message.ends_with('"') && self.message.len() > 1 {
                self.message = self.message[1..self.message.len()-1].to_string();
            }
        } else if self.message.is_empty() {
            // Fall back to first field if no "message" field
            let _ = write!(&mut self.message, "{:?}", value);
            if self.message.starts_with('"') && self.message.ends_with('"') && self.message.len() > 1 {
                self.message = self.message[1..self.message.len()-1].to_string();
            }
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" || self.message.is_empty() {
            self.message = value.to_string();
        }
    }
}

impl<S> Layer<S> for LogBufferLayer
where
    S: tracing::Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let metadata = event.metadata();
        let level = metadata.level().to_string().to_uppercase();
        let target = metadata.target();

        // Extract the source (last component of target)
        let source = target.split("::").last().unwrap_or(target);

        // Extract the message using our visitor
        let mut visitor = MessageVisitor::new();
        event.record(&mut visitor);

        let message = if visitor.message.is_empty() {
            format!("[{}]", target)
        } else {
            visitor.message
        };

        add_log_entry(&level, source, &message);
    }
}

/// Initialize the tracing subscriber with both fmt output and log buffer capture
pub fn init_tracing() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(LogBufferLayer)
        .init();
}

/// Get full diagnostic status
pub fn get_diagnostic_status(version: String) -> DiagnosticStatus {
    let cert_dir = cert_manager::get_cert_dir();
    let cert_path = cert_dir.join("localhost.crt");
    let key_path = cert_dir.join("localhost.key");

    let cert_exists = cert_path.exists() && key_path.exists();
    let cert_valid = if cert_exists {
        validate_cert_files(&cert_path, &key_path)
    } else {
        false
    };

    let cert_trusted = cert_manager::is_cert_trusted().unwrap_or(false);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let https_running = check_port_listening(HTTPS_PORT);
    let http_running = check_port_listening(HTTP_PORT);

    let overall_status = if https_running && http_running && cert_valid {
        if cfg!(target_os = "windows") && !cert_trusted {
            OverallStatus::Warning
        } else {
            OverallStatus::Ready
        }
    } else if http_running || https_running {
        OverallStatus::Warning
    } else {
        OverallStatus::Error
    };

    DiagnosticStatus {
        https_running,
        http_running,
        cert_exists,
        cert_valid,
        cert_trusted,
        cert_path: cert_dir.to_string_lossy().to_string(),
        version,
        uptime_seconds: now.saturating_sub(*APP_START_TIME),
        platform: std::env::consts::OS.to_string(),
        overall_status,
    }
}

/// Get certificate information
pub fn get_certificate_info() -> CertificateInfo {
    let cert_path = cert_manager::get_cert_path();

    let mut info = CertificateInfo {
        exists: cert_path.exists(),
        path: cert_path.to_string_lossy().to_string(),
        file_size_bytes: None,
        created: None,
        modified: None,
        is_trusted: cert_manager::is_cert_trusted().unwrap_or(false),
    };

    if info.exists {
        if let Ok(metadata) = fs::metadata(&cert_path) {
            info.file_size_bytes = Some(metadata.len());
            if let Ok(created) = metadata.created() {
                info.created = Some(format_system_time(created));
            }
            if let Ok(modified) = metadata.modified() {
                info.modified = Some(format_system_time(modified));
            }
        }
    }

    info
}

/// Test connections to both endpoints
pub async fn test_connections() -> ConnectionTestResult {
    let https_result = test_endpoint("https", HTTPS_PORT).await;
    let http_result = test_endpoint("http", HTTP_PORT).await;

    // Test localhost resolution
    let localhost_resolves = std::net::ToSocketAddrs::to_socket_addrs("localhost:80")
        .map(|mut addrs| addrs.next().is_some())
        .unwrap_or(false);

    // Test loopback accessibility
    let loopback_accessible = std::net::TcpListener::bind("127.0.0.1:0").is_ok();

    let success = https_result.0 || http_result.0;
    let message = if https_result.0 && http_result.0 {
        "Both connections working perfectly!".to_string()
    } else if https_result.0 {
        "HTTPS connection working (Safari compatible)".to_string()
    } else if http_result.0 {
        "HTTP connection working (use this for Chrome/Firefox/Edge)".to_string()
    } else {
        "Connection failed - servers may need restart".to_string()
    };

    ConnectionTestResult {
        success,
        https_ok: https_result.0,
        http_ok: http_result.0,
        https_latency_ms: https_result.1,
        http_latency_ms: http_result.1,
        localhost_resolves,
        loopback_accessible,
        message,
    }
}

async fn test_endpoint(protocol: &str, port: u16) -> (bool, Option<u64>) {
    let start = std::time::Instant::now();

    // Simple TCP connection test
    match tokio::time::timeout(
        std::time::Duration::from_secs(2),
        tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port)),
    )
    .await
    {
        Ok(Ok(_)) => (true, Some(start.elapsed().as_millis() as u64)),
        _ => (false, None),
    }
}

/// Get list of printers
pub fn get_printers() -> Vec<PrinterInfo> {
    printer::list_printers().unwrap_or_default()
}

/// Format diagnostics for clipboard copy
pub fn format_diagnostics_for_copy(status: &DiagnosticStatus, printers: &[PrinterInfo]) -> String {
    let mut output = String::new();
    output.push_str("=== AnyMobile Print Helper Diagnostics ===\n\n");
    output.push_str(&format!("Generated: {}\n", chrono::Local::now().format("%Y-%m-%d %H:%M:%S")));
    output.push_str(&format!("Version: {}\n", status.version));
    output.push_str(&format!("Platform: {}\n", status.platform));
    output.push_str(&format!("Uptime: {} seconds\n\n", status.uptime_seconds));

    output.push_str("Server Status:\n");
    output.push_str(&format!("  HTTPS ({}): {}\n", HTTPS_PORT, if status.https_running { "Running" } else { "Stopped" }));
    output.push_str(&format!("  HTTP ({}): {}\n", HTTP_PORT, if status.http_running { "Running" } else { "Stopped" }));

    output.push_str("\nCertificate:\n");
    output.push_str(&format!("  Path: {}\n", status.cert_path));
    output.push_str(&format!("  Exists: {}\n", status.cert_exists));
    output.push_str(&format!("  Valid: {}\n", status.cert_valid));
    output.push_str(&format!("  Trusted (Windows): {}\n", status.cert_trusted));

    output.push_str(&format!("\nPrinters ({} found):\n", printers.len()));
    for printer in printers {
        let default_marker = if printer.is_default { " (default)" } else { "" };
        output.push_str(&format!("  - {}{} [{}]\n", printer.name, default_marker, printer.status));
    }

    output.push_str("\nOverall Status: ");
    match status.overall_status {
        OverallStatus::Ready => output.push_str("Ready\n"),
        OverallStatus::Warning => output.push_str("Warning - needs attention\n"),
        OverallStatus::Error => output.push_str("Error - not working\n"),
    }

    output
}

/// Validate certificate files are not empty and properly formatted
fn validate_cert_files(cert_path: &PathBuf, key_path: &PathBuf) -> bool {
    match (fs::read(cert_path), fs::read(key_path)) {
        (Ok(cert), Ok(key)) => {
            !cert.is_empty()
                && !key.is_empty()
                && cert.starts_with(b"-----BEGIN CERTIFICATE-----")
        }
        _ => false,
    }
}

/// Check if a port is being listened on
fn check_port_listening(port: u16) -> bool {
    // If we can't bind, something else (our server) is using it
    std::net::TcpListener::bind(format!("127.0.0.1:{}", port)).is_err()
}

/// Format SystemTime as a readable string
fn format_system_time(time: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(time)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

/// Open the certificate folder in the file manager
pub fn open_cert_folder() -> Result<(), String> {
    let cert_dir = cert_manager::get_cert_dir();

    // Create directory if it doesn't exist
    if !cert_dir.exists() {
        fs::create_dir_all(&cert_dir).map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&cert_dir)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&cert_dir)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(&cert_dir)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Regenerate the self-signed certificate
pub fn regenerate_certificate() -> Result<(), String> {
    let cert_dir = cert_manager::get_cert_dir();
    let cert_path = cert_dir.join("localhost.crt");
    let key_path = cert_dir.join("localhost.key");

    // Delete existing files
    let _ = fs::remove_file(&cert_path);
    let _ = fs::remove_file(&key_path);

    tracing::info!("Certificate files deleted. New certificate will be generated on next server start.");

    // Note: The server will regenerate the certificate on next request
    // For immediate regeneration, we'd need to call server::get_or_create_certificate()
    // but that's private. A restart is the cleanest approach.

    Ok(())
}
