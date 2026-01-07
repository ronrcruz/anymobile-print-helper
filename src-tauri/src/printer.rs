//! Printer functionality - cross-platform PDF printing

use crate::server::PrinterInfo;
use std::process::Command;
use tempfile::NamedTempFile;
use std::io::Write;
use uuid::Uuid;

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
    // Use PowerShell to list printers
    let output = Command::new("powershell")
        .args([
            "-Command",
            "Get-Printer | Select-Object Name, Default, PrinterStatus | ConvertTo-Json",
        ])
        .output()?;

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

#[cfg(target_os = "windows")]
async fn print_pdf_windows(
    pdf_path: &str,
    printer_name: Option<&str>,
    copies: u32,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Try to find SumatraPDF in common locations
    let sumatra_paths = [
        "SumatraPDF.exe",
        "C:\\Program Files\\SumatraPDF\\SumatraPDF.exe",
        "C:\\Program Files (x86)\\SumatraPDF\\SumatraPDF.exe",
    ];

    let sumatra_path = sumatra_paths
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .map(|s| s.to_string());

    if let Some(sumatra) = sumatra_path {
        // Use SumatraPDF for reliable no-scale printing
        let mut args = vec![
            "-print-to".to_string(),
            printer_name.unwrap_or("default").to_string(),
            "-print-settings".to_string(),
            format!("noscale,paper=letter,{copies}x"),
            "-silent".to_string(),
            pdf_path.to_string(),
        ];

        if printer_name.is_none() {
            args[1] = "default".to_string();
            args.remove(0);
            args.remove(0);
            args.insert(0, "-print-to-default".to_string());
        }

        let status = Command::new(&sumatra)
            .args(&args)
            .status()?;

        if !status.success() {
            return Err("SumatraPDF print command failed".into());
        }
    } else {
        // Fallback: Use Windows print verb with shell execute
        // This will use the system's default PDF handler
        let status = Command::new("cmd")
            .args(["/C", "start", "/wait", "", "/print", pdf_path])
            .status()?;

        if !status.success() {
            return Err("Windows print command failed".into());
        }
    }

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
