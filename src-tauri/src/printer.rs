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

/// Get the directory where Ghostscript should be stored
#[cfg(target_os = "windows")]
fn get_ghostscript_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("anymobile-print-helper")
        .join("tools")
        .join("gs")
}

/// Get path to Ghostscript executable
#[cfg(target_os = "windows")]
fn get_ghostscript_path() -> PathBuf {
    get_ghostscript_dir().join("bin").join("gswin64c.exe")
}

/// Check if Ghostscript is installed and return the path if found
#[cfg(target_os = "windows")]
fn find_ghostscript_path() -> Option<PathBuf> {
    let gs_dir = get_ghostscript_dir();

    // Check direct path first
    let direct_path = gs_dir.join("bin").join("gswin64c.exe");
    if direct_path.exists() {
        return Some(direct_path);
    }

    // Check versioned subdirectory (installer creates gs10.04.0/bin/gswin64c.exe)
    let versioned_path = gs_dir.join("gs10.04.0").join("bin").join("gswin64c.exe");
    if versioned_path.exists() {
        return Some(versioned_path);
    }

    None
}

/// Download and install Ghostscript if not present
/// Ghostscript's mswinpr2 device respects DEVMODE settings unlike SumatraPDF
#[cfg(target_os = "windows")]
pub async fn ensure_ghostscript_available() -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    // Check if already installed
    if let Some(existing_path) = find_ghostscript_path() {
        tracing::info!("Ghostscript already available at {:?}", existing_path);
        return Ok(existing_path);
    }

    tracing::info!("Downloading Ghostscript for high-quality printing...");

    let gs_dir = get_ghostscript_dir();
    std::fs::create_dir_all(&gs_dir)?;

    // Download Ghostscript installer (64-bit)
    // Using the official GitHub releases from Artifex
    let download_url = "https://github.com/ArtifexSoftware/ghostpdl-downloads/releases/download/gs10040/gs10040w64.exe";
    let installer_path = gs_dir.join("gs_installer.exe");

    tracing::info!("Downloading from: {}", download_url);
    let response = reqwest::get(download_url).await?;

    if !response.status().is_success() {
        return Err(format!("Failed to download Ghostscript: HTTP {}", response.status()).into());
    }

    let bytes = response.bytes().await?;
    std::fs::write(&installer_path, &bytes)?;
    tracing::info!("Downloaded installer to {:?}", installer_path);

    // Run silent install to our local directory
    // /S = silent, /D = destination directory
    // NOTE: We do NOT use CREATE_NO_WINDOW because it hides the UAC prompt!
    let install_target = gs_dir.to_string_lossy().to_string();
    tracing::info!("Installing Ghostscript to: {} (UAC prompt will appear)", install_target);

    let mut child = Command::new(&installer_path)
        .args(["/S", &format!("/D={}", install_target)])
        // NO creation_flags - let the installer show UAC prompt
        .spawn()?;

    // Wait for installer to complete (with timeout)
    let timeout = std::time::Duration::from_secs(120);
    let start = std::time::Instant::now();
    loop {
        match child.try_wait()? {
            Some(status) => {
                tracing::info!("Installer completed with status: {:?}", status);
                break;
            }
            None if start.elapsed() > timeout => {
                let _ = child.kill();
                tracing::error!("Installer timed out after 120 seconds");
                return Err("Ghostscript installer timed out".into());
            }
            None => {
                // Still running, wait a bit
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }
        }
    }

    // Give the installer a moment to finish writing files
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Clean up installer
    let _ = std::fs::remove_file(&installer_path);

    // Verify installation using the helper function
    if let Some(installed_path) = find_ghostscript_path() {
        tracing::info!("Ghostscript installed successfully to {:?}", installed_path);
        Ok(installed_path)
    } else {
        Err("Ghostscript installation failed - executable not found".into())
    }
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

/// Configure printer for high-quality label printing (600 DPI, Matte paper)
/// This modifies the printer's DEVMODE settings before SumatraPDF prints
#[cfg(target_os = "windows")]
fn configure_printer_quality(printer_name: &str) -> Result<(), String> {
    tracing::info!("Configuring printer quality for: {}", printer_name);

    // PowerShell script that:
    // 1. Queries supported media types from the printer
    // 2. Finds "Matte" or "Premium" paper type
    // 3. Sets 600 DPI and the discovered media type via DEVMODE
    let ps_script = format!(r#"
$ErrorActionPreference = 'Stop'
$printerName = '{}'

# Add .NET types for printer configuration
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public class PrinterConfig {{
    [DllImport("winspool.drv", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern bool OpenPrinter(string pPrinterName, out IntPtr hPrinter, IntPtr pDefault);

    [DllImport("winspool.drv", CharSet = CharSet.Unicode)]
    public static extern int DocumentProperties(IntPtr hwnd, IntPtr hPrinter, string pDeviceName,
        IntPtr pDevModeOutput, IntPtr pDevModeInput, int fMode);

    [DllImport("winspool.drv", SetLastError = true)]
    public static extern bool ClosePrinter(IntPtr hPrinter);

    [DllImport("winspool.drv", CharSet = CharSet.Unicode)]
    public static extern int DeviceCapabilities(string pDevice, string pPort,
        ushort fwCapability, IntPtr pOutput, IntPtr pDevMode);
}}
'@

# DeviceCapabilities constants
$DC_MEDIATYPES = 35
$DC_MEDIATYPENAMES = 36

# DEVMODE field offsets (64-bit Windows)
$dmFields_offset = 40
$dmPrintQuality_offset = 58
$dmYResolution_offset = 60
$dmMediaType_offset = 62

# DEVMODE field flags
$DM_PRINTQUALITY = 0x0400
$DM_YRESOLUTION = 0x2000
$DM_MEDIATYPE = 0x0200

# Step 1: Query supported media types
$mediaTypeId = 0  # Default to plain paper
$count = [PrinterConfig]::DeviceCapabilities($printerName, $null, $DC_MEDIATYPES, [IntPtr]::Zero, [IntPtr]::Zero)

if ($count -gt 0) {{
    Write-Host "Found $count media types"

    # Allocate buffers for IDs (4 bytes each) and names (64 chars * 2 bytes = 128 bytes each)
    $idBuffer = [System.Runtime.InteropServices.Marshal]::AllocHGlobal($count * 4)
    $nameBuffer = [System.Runtime.InteropServices.Marshal]::AllocHGlobal($count * 128)

    try {{
        [PrinterConfig]::DeviceCapabilities($printerName, $null, $DC_MEDIATYPES, $idBuffer, [IntPtr]::Zero) | Out-Null
        [PrinterConfig]::DeviceCapabilities($printerName, $null, $DC_MEDIATYPENAMES, $nameBuffer, [IntPtr]::Zero) | Out-Null

        # Find best match: Premium Matte > Presentation Matte > Matte > any
        $bestMatch = $null
        $bestPriority = 0

        for ($i = 0; $i -lt $count; $i++) {{
            $id = [System.Runtime.InteropServices.Marshal]::ReadInt32($idBuffer, $i * 4)
            $namePtr = [IntPtr]::Add($nameBuffer, $i * 128)
            $name = [System.Runtime.InteropServices.Marshal]::PtrToStringUni($namePtr, 64).TrimEnd([char]0)

            Write-Host "  Media type $i : ID=$id, Name='$name'"

            # Priority matching
            if ($name -match "Premium.*Presentation.*Matte" -and $bestPriority -lt 4) {{
                $bestMatch = $id
                $bestPriority = 4
                Write-Host "    -> Best match (priority 4)"
            }} elseif ($name -match "Premium.*Matte" -and $bestPriority -lt 3) {{
                $bestMatch = $id
                $bestPriority = 3
                Write-Host "    -> Match (priority 3)"
            }} elseif ($name -match "Presentation.*Matte" -and $bestPriority -lt 2) {{
                $bestMatch = $id
                $bestPriority = 2
                Write-Host "    -> Match (priority 2)"
            }} elseif ($name -match "Matte" -and $bestPriority -lt 1) {{
                $bestMatch = $id
                $bestPriority = 1
                Write-Host "    -> Match (priority 1)"
            }}
        }}

        if ($bestMatch -ne $null) {{
            $mediaTypeId = $bestMatch
            Write-Host "Selected media type ID: $mediaTypeId"
        }} else {{
            Write-Host "No matte paper found, using default (0)"
        }}
    }} finally {{
        [System.Runtime.InteropServices.Marshal]::FreeHGlobal($idBuffer)
        [System.Runtime.InteropServices.Marshal]::FreeHGlobal($nameBuffer)
    }}
}}

# Step 2: Open printer and modify DEVMODE
$hPrinter = [IntPtr]::Zero
if ([PrinterConfig]::OpenPrinter($printerName, [ref]$hPrinter, [IntPtr]::Zero)) {{
    try {{
        # Get DEVMODE size
        $size = [PrinterConfig]::DocumentProperties([IntPtr]::Zero, $hPrinter, $printerName, [IntPtr]::Zero, [IntPtr]::Zero, 0)
        Write-Host "DEVMODE size: $size bytes"

        if ($size -gt 0) {{
            $pDevMode = [System.Runtime.InteropServices.Marshal]::AllocHGlobal($size)

            try {{
                # Get current DEVMODE (DM_OUT_BUFFER = 2)
                $result = [PrinterConfig]::DocumentProperties([IntPtr]::Zero, $hPrinter, $printerName, $pDevMode, [IntPtr]::Zero, 2)

                if ($result -ge 0) {{
                    # Read and modify dmFields
                    $dmFields = [System.Runtime.InteropServices.Marshal]::ReadInt32($pDevMode, $dmFields_offset)
                    $dmFields = $dmFields -bor $DM_PRINTQUALITY -bor $DM_YRESOLUTION -bor $DM_MEDIATYPE
                    [System.Runtime.InteropServices.Marshal]::WriteInt32($pDevMode, $dmFields_offset, $dmFields)

                    # Set 600 DPI
                    [System.Runtime.InteropServices.Marshal]::WriteInt16($pDevMode, $dmPrintQuality_offset, 600)
                    [System.Runtime.InteropServices.Marshal]::WriteInt16($pDevMode, $dmYResolution_offset, 600)

                    # Set media type
                    [System.Runtime.InteropServices.Marshal]::WriteInt16($pDevMode, $dmMediaType_offset, $mediaTypeId)

                    # Apply settings (DM_IN_BUFFER | DM_OUT_BUFFER = 10)
                    $result = [PrinterConfig]::DocumentProperties([IntPtr]::Zero, $hPrinter, $printerName, $pDevMode, $pDevMode, 10)

                    if ($result -eq 1) {{
                        Write-Host "SUCCESS: Configured 600 DPI, MediaType=$mediaTypeId"
                    }} else {{
                        Write-Host "WARNING: DocumentProperties returned $result (settings may not persist)"
                    }}
                }} else {{
                    Write-Host "ERROR: Failed to get DEVMODE (result=$result)"
                }}
            }} finally {{
                [System.Runtime.InteropServices.Marshal]::FreeHGlobal($pDevMode)
            }}
        }}
    }} finally {{
        [PrinterConfig]::ClosePrinter($hPrinter) | Out-Null
    }}
}} else {{
    Write-Host "ERROR: Could not open printer '$printerName'"
}}
"#, printer_name);

    let output = Command::new("powershell")
        .args(["-ExecutionPolicy", "Bypass", "-NoProfile", "-Command", &ps_script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("PowerShell failed: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    tracing::info!("Printer config output:\n{}", stdout);
    if !stderr.is_empty() {
        tracing::warn!("Printer config stderr:\n{}", stderr);
    }

    if stdout.contains("SUCCESS") {
        Ok(())
    } else if stdout.contains("WARNING") {
        // Settings applied but may not persist - continue anyway
        Ok(())
    } else {
        Err(format!("Failed to configure printer: {}", stdout))
    }
}

/// Print PDF using Ghostscript (high quality - respects DEVMODE)
#[cfg(target_os = "windows")]
async fn print_pdf_ghostscript(
    pdf_path: &str,
    printer_name: Option<&str>,
    copies: u32,
    gs_path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing::info!("=== WINDOWS PRINT (Ghostscript) ===");
    tracing::info!("PDF path: {}", pdf_path);
    tracing::info!("Printer: {:?}", printer_name);
    tracing::info!("Copies: {}", copies);
    tracing::info!("Ghostscript path: {:?}", gs_path);

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

    // Configure DEVMODE for Epson printers (Ghostscript will use these settings!)
    // Unlike SumatraPDF, Ghostscript's mswinpr2 device respects the printer's DEVMODE
    if printer.to_lowercase().contains("epson") {
        tracing::info!("Detected Epson printer - configuring DEVMODE for high-quality printing...");
        match configure_printer_quality(&printer) {
            Ok(()) => tracing::info!("Printer DEVMODE configured: 600 DPI + Matte paper"),
            Err(e) => tracing::warn!("Could not configure DEVMODE: {}. Using defaults.", e),
        }
    }

    // Build Ghostscript command
    // mswinpr2 device: Windows printer driver that respects DEVMODE settings
    // Key difference from SumatraPDF: Ghostscript queries and uses the printer's configured DEVMODE
    let output_device = format!("%printer%{}", printer);

    let args = vec![
        "-dBATCH".to_string(),           // Exit after processing
        "-dNOPAUSE".to_string(),         // No pause between pages
        "-dPrinted".to_string(),         // Suppress showpage
        "-dNoCancel".to_string(),        // Don't show cancel dialog
        "-dNOSAFER".to_string(),         // Allow file operations (needed for some PDFs)
        "-sDEVICE=mswinpr2".to_string(), // Windows printer device (uses DEVMODE!)
        format!("-sOutputFile={}", output_device),
        "-dPDFFitPage=false".to_string(), // Don't fit to page (actual size)
        "-dPSFitPage=false".to_string(),  // Don't fit PostScript to page
        format!("-dNumCopies={}", copies), // Handle copies in one command
        pdf_path.to_string(),
    ];

    tracing::info!("Ghostscript args: {:?}", args);

    let output = Command::new(&gs_path)
        .args(&args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;

    tracing::info!("Ghostscript exit status: {:?}", output.status);

    if !output.stdout.is_empty() {
        tracing::debug!("stdout: {}", String::from_utf8_lossy(&output.stdout));
    }
    if !output.stderr.is_empty() {
        // Ghostscript often outputs to stderr even on success
        let stderr_str = String::from_utf8_lossy(&output.stderr);
        if output.status.success() {
            tracing::debug!("stderr: {}", stderr_str);
        } else {
            tracing::error!("stderr: {}", stderr_str);
        }
    }

    if !output.status.success() {
        let err_msg = format!(
            "Ghostscript print failed (exit code {:?}): {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
        tracing::error!("{}", err_msg);
        return Err(err_msg.into());
    }

    tracing::info!("=== WINDOWS PRINT COMPLETE ===");
    Ok(())
}

/// Print PDF using SumatraPDF (fallback - lower quality, ignores DEVMODE)
#[cfg(target_os = "windows")]
async fn print_pdf_sumatra(
    pdf_path: &str,
    printer_name: Option<&str>,
    copies: u32,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing::info!("=== WINDOWS PRINT (SumatraPDF fallback) ===");
    tracing::info!("PDF path: {}", pdf_path);
    tracing::info!("Printer: {:?}", printer_name);
    tracing::info!("Copies: {}", copies);

    // Ensure SumatraPDF is available (download if needed)
    let sumatra_path = ensure_sumatra_available().await?;
    tracing::info!("SumatraPDF path: {:?}", sumatra_path);

    // Build print command
    let mut args = vec![
        "-print-to".to_string(),
    ];

    if let Some(name) = printer_name {
        args.push(name.to_string());
    } else {
        args.push("-print-to-default".to_string());
        args.remove(0); // Remove -print-to, use -print-to-default instead
        args.insert(0, "-print-to-default".to_string());
    }

    // Add settings for multiple copies and quality
    args.push("-print-settings".to_string());
    args.push(format!("{}x,noscale", copies));
    args.push("-silent".to_string());
    args.push(pdf_path.to_string());

    tracing::info!("SumatraPDF args: {:?}", args);

    let output = Command::new(&sumatra_path)
        .args(&args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;

    tracing::info!("SumatraPDF exit status: {:?}", output.status);

    if !output.status.success() {
        let err_msg = format!(
            "SumatraPDF print failed (exit code {:?})",
            output.status.code()
        );
        tracing::error!("{}", err_msg);
        return Err(err_msg.into());
    }

    tracing::info!("=== WINDOWS PRINT COMPLETE (SumatraPDF) ===");
    Ok(())
}

/// Main Windows print function - uses Ghostscript if available, falls back to SumatraPDF
#[cfg(target_os = "windows")]
async fn print_pdf_windows(
    pdf_path: &str,
    printer_name: Option<&str>,
    copies: u32,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Check if Ghostscript is installed (was downloaded at app startup)
    if let Some(gs_path) = find_ghostscript_path() {
        tracing::info!("Using Ghostscript for high-quality printing");
        print_pdf_ghostscript(pdf_path, printer_name, copies, &gs_path).await
    } else {
        tracing::warn!("Ghostscript not installed, using SumatraPDF (lower quality)");
        tracing::warn!("For best print quality, please restart the app and accept the Ghostscript installation prompt");
        print_pdf_sumatra(pdf_path, printer_name, copies).await
    }
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
