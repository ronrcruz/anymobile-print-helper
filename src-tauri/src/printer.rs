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
    // Use PowerShell with .NET to print PDF at actual size
    // This works without any external dependencies
    let printer_arg = match printer_name {
        Some(name) => format!("'{}'", name),
        None => "(Get-WmiObject -Query \"SELECT * FROM Win32_Printer WHERE Default=$true\").Name".to_string(),
    };

    // PowerShell script that prints PDF at 100% scale using Adobe Reader if available,
    // otherwise falls back to Windows' built-in PDF printing
    let ps_script = format!(r#"
$pdfPath = '{pdf_path}'
$printerName = {printer_arg}
$copies = {copies}

# Try Adobe Reader first (most reliable for exact scaling)
$adobePaths = @(
    "$env:ProgramFiles\Adobe\Acrobat Reader DC\Reader\AcroRd32.exe",
    "$env:ProgramFiles\Adobe\Acrobat DC\Acrobat\Acrobat.exe",
    "${env:ProgramFiles(x86)}\Adobe\Acrobat Reader DC\Reader\AcroRd32.exe"
)

$adobePath = $adobePaths | Where-Object {{ Test-Path $_ }} | Select-Object -First 1

if ($adobePath) {{
    # Adobe Reader: /t prints to specific printer, /s suppresses splash
    for ($i = 0; $i -lt $copies; $i++) {{
        Start-Process -FilePath $adobePath -ArgumentList "/t", "`"$pdfPath`"", "`"$printerName`"" -Wait -WindowStyle Hidden
    }}
}} else {{
    # Fallback: Use Windows default PDF handler with print verb
    # Set registry to disable "fit to page" for Microsoft Print to PDF if it's the handler
    for ($i = 0; $i -lt $copies; $i++) {{
        Start-Process -FilePath $pdfPath -Verb Print -Wait
    }}
}}
"#);

    let status = Command::new("powershell")
        .args(["-ExecutionPolicy", "Bypass", "-Command", &ps_script])
        .status()?;

    if !status.success() {
        return Err("Windows print command failed".into());
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
