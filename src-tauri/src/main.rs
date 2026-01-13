// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod server;
mod printer;
mod cert_manager;
mod diagnostics;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_updater::UpdaterExt;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Application state shared across the app
pub struct AppState {
    pub server_running: bool,
    pub last_print_job: Option<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            server_running: false,
            last_print_job: None,
        }
    }
}

// =====================
// TAURI COMMANDS
// =====================

/// Get diagnostic status
#[tauri::command]
async fn get_diagnostics(app: tauri::AppHandle) -> Result<diagnostics::DiagnosticStatus, String> {
    let version = app.package_info().version.to_string();
    Ok(diagnostics::get_diagnostic_status(version))
}

/// Test connection to both endpoints
#[tauri::command]
async fn test_connection() -> Result<diagnostics::ConnectionTestResult, String> {
    Ok(diagnostics::test_connections().await)
}

/// Get list of available printers
#[tauri::command]
async fn get_printers() -> Result<Vec<server::PrinterInfo>, String> {
    Ok(diagnostics::get_printers())
}

/// Get certificate information
#[tauri::command]
async fn get_certificate_info() -> Result<diagnostics::CertificateInfo, String> {
    Ok(diagnostics::get_certificate_info())
}

/// Check if certificate is trusted on Windows
#[tauri::command]
fn check_cert_trusted() -> Result<bool, String> {
    cert_manager::is_cert_trusted()
}

/// Install certificate to Windows store
#[tauri::command]
fn install_certificate(use_admin: bool) -> Result<(), String> {
    if use_admin {
        cert_manager::install_cert_local_machine()
    } else {
        cert_manager::install_cert_current_user()
    }
}

/// Regenerate the certificate
#[tauri::command]
fn regenerate_certificate() -> Result<String, String> {
    diagnostics::regenerate_certificate()?;
    Ok("Certificate deleted. Restart the app to generate a new one.".to_string())
}

/// Open certificate folder
#[tauri::command]
fn open_cert_folder() -> Result<(), String> {
    diagnostics::open_cert_folder()
}

/// Get recent logs
#[tauri::command]
fn get_recent_logs(count: Option<usize>, level: Option<String>) -> Vec<diagnostics::LogEntry> {
    diagnostics::get_recent_logs(count, level.as_deref())
}

/// Clear log buffer
#[tauri::command]
fn clear_logs() {
    diagnostics::clear_logs()
}

/// Copy diagnostics to clipboard format
#[tauri::command]
async fn copy_diagnostics(app: tauri::AppHandle) -> Result<String, String> {
    let version = app.package_info().version.to_string();
    let status = diagnostics::get_diagnostic_status(version);
    let printers = diagnostics::get_printers();
    Ok(diagnostics::format_diagnostics_for_copy(&status, &printers))
}

/// Get current platform
#[tauri::command]
fn get_platform() -> String {
    std::env::consts::OS.to_string()
}

fn main() {
    // Install rustls crypto provider (required for rustls 0.23+)
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    // Initialize logging with custom layer that captures to in-memory buffer
    diagnostics::init_tracing();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Arc::new(Mutex::new(AppState::default())))
        .invoke_handler(tauri::generate_handler![
            get_diagnostics,
            test_connection,
            get_printers,
            get_certificate_info,
            check_cert_trusted,
            install_certificate,
            regenerate_certificate,
            open_cert_folder,
            get_recent_logs,
            clear_logs,
            copy_diagnostics,
            get_platform
        ])
        .setup(|app| {
            // Create system tray menu
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let show = MenuItem::with_id(app, "show", "Show Status", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;

            // Create tray icon
            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => {
                        app.exit(0);
                    }
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            // Start HTTP server in background
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = server::start_server(app_handle).await {
                    tracing::error!("Failed to start HTTP server: {}", e);
                }
            });

            // Pre-download Ghostscript on Windows for high-quality printing
            // This happens at startup so user doesn't wait during print
            #[cfg(target_os = "windows")]
            {
                tauri::async_runtime::spawn(async {
                    tracing::info!("Checking Ghostscript availability for high-quality printing...");
                    match printer::ensure_ghostscript_available().await {
                        Ok(path) => tracing::info!("Ghostscript ready at: {:?}", path),
                        Err(e) => tracing::warn!("Ghostscript setup failed: {}. Will use SumatraPDF as fallback.", e),
                    }
                });
            }

            // Hide window on startup if minimized flag is set
            if std::env::args().any(|arg| arg == "--minimized") {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }

            // Check for updates in background
            let update_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // Wait a bit for app to fully initialize
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

                match update_handle.updater() {
                    Ok(updater) => {
                        match updater.check().await {
                            Ok(Some(update)) => {
                                tracing::info!(
                                    "Update available: {} -> {}",
                                    update.current_version,
                                    update.version
                                );
                                // Download and install the update
                                match update.download_and_install(|_, _| {}, || {}).await {
                                    Ok(_) => {
                                        tracing::info!("Update installed successfully. Restart to apply.");
                                    }
                                    Err(e) => {
                                        tracing::warn!("Failed to install update: {}", e);
                                    }
                                }
                            }
                            Ok(None) => {
                                tracing::info!("App is up to date");
                            }
                            Err(e) => {
                                tracing::warn!("Failed to check for updates: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Updater not available: {}", e);
                    }
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
