//! Printer functionality - cross-platform PDF printing

use crate::server::PrinterInfo;
use std::process::Command;
use tempfile::NamedTempFile;
use std::io::Write;
use std::path::PathBuf;
use uuid::Uuid;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

/// Windows flag to hide console window
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// List available printers on the system
pub fn list_printers() -> Result<Vec<PrinterInfo>, Box<dyn std::error::Error>> {
    #[cfg(target_os = "windows")]
    {
        list_printers_windows()
    }

    #[cfg(target_os = "macos")]
    {
        list_printers_macos()
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Ok(vec![])
    }
}

/// Print a PDF file
pub async fn print_pdf(
    pdf_data: &[u8],
    printer_name: Option<&str>,
    copies: u32,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Save PDF to temp file
    let mut temp_file = NamedTempFile::with_suffix(".pdf")?;
    temp_file.write_all(pdf_data)?;
    let temp_path = temp_file.path().to_string_lossy().to_string();

    // Generate job ID
    let job_id = Uuid::new_v4().to_string();

    #[cfg(target_os = "windows")]
    {
        print_pdf_windows(&temp_path, printer_name, copies).await?;
    }

    #[cfg(target_os = "macos")]
    {
        print_pdf_macos(&temp_path, printer_name, copies).await?;
    }

    // Keep temp file alive until print job is queued
    // (it will be deleted when temp_file goes out of scope after a delay)
    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

    Ok(job_id)
}

// ============================================================================
// Windows Implementation
// ============================================================================

#[cfg(target_os = "windows")]
fn list_printers_windows() -> Result<Vec<PrinterInfo>, Box<dyn std::error::Error>> {
    tracing::info!("Listing printers on Windows...");

    // Use PowerShell to list printers
    let output = Command::new("powershell")
        .args([
            "-Command",
            "Get-Printer | Select-Object Name, Default, PrinterStatus | ConvertTo-Json",
        ])
        .output()?;

    tracing::info!("PowerShell exit status: {:?}", output.status);
    tracing::debug!("PowerShell stdout: {}", String::from_utf8_lossy(&output.stdout));
    tracing::debug!("PowerShell stderr: {}", String::from_utf8_lossy(&output.stderr));

    if !output.status.success() {
        return Ok(vec![]);
    }

    let json_str = String::from_utf8_lossy(&output.stdout);

    // Handle both single printer and array of printers
    let printers: Vec<PrinterInfo> = if json_str.trim().starts_with('[') {
        #[derive(serde::Deserialize)]
        struct WinPrinter {
            Name: String,
            Default: Option<bool>,
            PrinterStatus: Option<u32>,
        }

        let win_printers: Vec<WinPrinter> = serde_json::from_str(&json_str).unwrap_or_default();
        win_printers
            .into_iter()
            .map(|p| PrinterInfo {
                name: p.Name,
                is_default: p.Default.unwrap_or(false),
                status: match p.PrinterStatus.unwrap_or(0) {
                    0 => "ready".to_string(),
                    1 => "busy".to_string(),
                    _ => "unknown".to_string(),
                },
            })
            .collect()
    } else if json_str.trim().starts_with('{') {
        #[derive(serde::Deserialize)]
        struct WinPrinter {
            Name: String,
            Default: Option<bool>,
            PrinterStatus: Option<u32>,
        }

        if let Ok(p) = serde_json::from_str::<WinPrinter>(&json_str) {
            vec![PrinterInfo {
                name: p.Name,
                is_default: p.Default.unwrap_or(false),
                status: match p.PrinterStatus.unwrap_or(0) {
                    0 => "ready".to_string(),
                    1 => "busy".to_string(),
                    _ => "unknown".to_string(),
                },
            }]
        } else {
            vec![]
        }
    } else {
        vec![]
    };

    Ok(printers)
}

/// Get the path where SumatraPDF should be stored
#[cfg(target_os = "windows")]
fn get_sumatra_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("anymobile-print-helper")
        .join("tools")
}

/// Get path to SumatraPDF executable
#[cfg(target_os = "windows")]
fn get_sumatra_path() -> PathBuf {
    get_sumatra_dir().join("SumatraPDF.exe")
}

/// Download SumatraPDF if not present
#[cfg(target_os = "windows")]
async fn ensure_sumatra_available() -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let sumatra_path = get_sumatra_path();

    if sumatra_path.exists() {
        tracing::info!("SumatraPDF already available at {:?}", sumatra_path);
        return Ok(sumatra_path);
    }

    tracing::info!("Downloading SumatraPDF for silent printing...");

    // Create directory
    let sumatra_dir = get_sumatra_dir();
    std::fs::create_dir_all(&sumatra_dir)?;

    // Download SumatraPDF portable (64-bit)
    // Using the official SourceForge mirror for the portable version
    let download_url = "https://www.sumatrapdfreader.org/dl/rel/3.5.2/SumatraPDF-3.5.2-64.exe";

    let response = reqwest::get(download_url).await?;

    if !response.status().is_success() {
        return Err(format!("Failed to download SumatraPDF: HTTP {}", response.status()).into());
    }

    let bytes = response.bytes().await?;
    std::fs::write(&sumatra_path, &bytes)?;

    tracing::info!("SumatraPDF downloaded successfully to {:?}", sumatra_path);
    Ok(sumatra_path)
}

#[cfg(target_os = "windows")]
async fn print_pdf_windows(
    pdf_path: &str,
    printer_name: Option<&str>,
    copies: u32,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing::info!("=== WINDOWS PRINT (SumatraPDF) ===");
    tracing::info!("PDF path: {}", pdf_path);
    tracing::info!("Printer: {:?}", printer_name);
    tracing::info!("Copies: {}", copies);

    // Ensure SumatraPDF is available
    let sumatra_path = ensure_sumatra_available().await?;

    // Get printer name (use default if not specified)
    let printer = match printer_name {
        Some(name) => name.to_string(),
        None => {
            // Get default printer via PowerShell
            let output = Command::new("powershell")
                .args(["-Command", "(Get-WmiObject -Query \"SELECT * FROM Win32_Printer WHERE Default=$true\").Name"])
                .creation_flags(CREATE_NO_WINDOW)
                .output()?;
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
    };

    tracing::info!("Using printer: {}", printer);

    // Build print settings for SumatraPDF
    // noscale = actual size (no fit-to-page)
    // For multiple copies, we call SumatraPDF multiple times
    // SumatraPDF print settings: https://www.sumatrapdfreader.org/docs/Command-line-arguments
    let print_settings = "noscale";

    for i in 0..copies {
        tracing::info!("Printing copy {} of {}", i + 1, copies);

        let output = Command::new(&sumatra_path)
            .args([
                "-print-to", &printer,
                "-print-settings", print_settings,
                "-silent",
                pdf_path,
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output()?;

        tracing::info!("SumatraPDF exit status: {:?}", output.status);

        if !output.stdout.is_empty() {
            tracing::debug!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        }
        if !output.stderr.is_empty() {
            tracing::debug!("stderr: {}", String::from_utf8_lossy(&output.stderr));
        }

        if !output.status.success() {
            let err_msg = format!(
                "SumatraPDF print failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            tracing::error!("{}", err_msg);
            return Err(err_msg.into());
        }
    }

    tracing::info!("=== WINDOWS PRINT COMPLETE ===");
    Ok(())
}

// ============================================================================
// macOS Implementation
// ============================================================================

#[cfg(target_os = "macos")]
fn list_printers_macos() -> Result<Vec<PrinterInfo>, Box<dyn std::error::Error>> {
    let output = Command::new("lpstat")
        .args(["-p", "-d"])
        .output()?;

    if !output.status.success() {
        return Ok(vec![]);
    }

    let output_str = String::from_utf8_lossy(&output.stdout);
    let mut printers = Vec::new();
    let mut default_printer = String::new();

    for line in output_str.lines() {
        if line.starts_with("printer ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let name = parts[1].to_string();
                let status = if line.contains("idle") {
                    "ready"
                } else if line.contains("printing") {
                    "busy"
                } else {
                    "unknown"
                };
                printers.push(PrinterInfo {
                    name,
                    is_default: false,
                    status: status.to_string(),
                });
            }
        } else if line.starts_with("system default destination:") {
            default_printer = line
                .replace("system default destination:", "")
                .trim()
                .to_string();
        }
    }

    // Mark default printer
    for printer in &mut printers {
        if printer.name == default_printer {
            printer.is_default = true;
        }
    }

    Ok(printers)
}

#[cfg(target_os = "macos")]
async fn print_pdf_macos(
    pdf_path: &str,
    printer_name: Option<&str>,
    copies: u32,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut args = vec![
        "-n".to_string(),
        copies.to_string(),
        "-o".to_string(),
        "fit-to-page=false".to_string(),
        "-o".to_string(),
        "scaling=100".to_string(),
    ];

    // Apply printer-specific high-quality settings for labels
    if let Some(printer) = printer_name {
        let printer_lower = printer.to_lowercase();

        if printer_lower.contains("epson") {
            // Epson-specific: 600 DPI, highest quality, premium matte paper
            args.extend([
                "-o".to_string(), "Resolution=600x600dpi".to_string(),
                "-o".to_string(), "EPIJ_Qual=307".to_string(),
                "-o".to_string(), "EPIJ_Medi=12".to_string(),  // Premium Presentation Paper Matte
            ]);
            tracing::info!("Applying Epson high-quality settings: 600dpi, quality=307, matte paper");
        } else if printer_lower.contains("hp") || printer_lower.contains("laserjet") {
            // HP-specific: use labels media type
            args.extend([
                "-o".to_string(), "MediaType=labels".to_string(),
            ]);
            tracing::info!("Applying HP settings: labels media type");
        } else {
            // Generic fallback: try common CUPS high quality option
            args.extend([
                "-o".to_string(), "print-quality=5".to_string(),
            ]);
            tracing::info!("Applying generic high-quality setting");
        }

        args.push("-d".to_string());
        args.push(printer.to_string());
    }

    args.push(pdf_path.to_string());

    tracing::info!("Executing lp with args: {:?}", args);

    let status = Command::new("lp")
        .args(&args)
        .status()?;

    if !status.success() {
        return Err("lp print command failed".into());
    }

    Ok(())
}
