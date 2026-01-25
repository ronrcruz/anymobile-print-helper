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

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        list_printers_unix()
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
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

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        print_pdf_unix(&temp_path, printer_name, copies).await?;
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
/// Checks: app local dir, Program Files, Program Files (x86), and PATH
#[cfg(target_os = "windows")]
fn find_ghostscript_path() -> Option<PathBuf> {
    // 1. Check app's local directory first
    let gs_dir = get_ghostscript_dir();

    let local_paths = [
        gs_dir.join("bin").join("gswin64c.exe"),
        gs_dir.join("gs10.04.0").join("bin").join("gswin64c.exe"),
    ];

    for path in &local_paths {
        if path.exists() {
            tracing::debug!("Found Ghostscript at app local: {:?}", path);
            return Some(path.clone());
        }
    }

    // 2. Check Program Files (standard manual install location)
    let program_files = std::env::var("ProgramFiles").unwrap_or_else(|_| "C:\\Program Files".to_string());
    let program_files_x86 = std::env::var("ProgramFiles(x86)").unwrap_or_else(|_| "C:\\Program Files (x86)".to_string());

    // Common Ghostscript versions to check
    let gs_versions = ["gs10.04.0", "gs10.03.1", "gs10.03.0", "gs10.02.1", "gs10.02.0", "gs10.01.2", "gs10.00.0", "gs9.56.1", "gs9.55.0"];

    for base in [&program_files, &program_files_x86] {
        let gs_base = PathBuf::from(base).join("gs");

        // Check versioned directories
        for version in &gs_versions {
            let path = gs_base.join(version).join("bin").join("gswin64c.exe");
            if path.exists() {
                tracing::debug!("Found Ghostscript at Program Files: {:?}", path);
                return Some(path);
            }
            // Also check 32-bit executable
            let path32 = gs_base.join(version).join("bin").join("gswin32c.exe");
            if path32.exists() {
                tracing::debug!("Found Ghostscript (32-bit) at Program Files: {:?}", path32);
                return Some(path32);
            }
        }
    }

    // 3. Check if gswin64c is in PATH
    if let Ok(output) = Command::new("where")
        .arg("gswin64c.exe")
        .output()
    {
        if output.status.success() {
            let path_str = String::from_utf8_lossy(&output.stdout);
            if let Some(first_line) = path_str.lines().next() {
                let path = PathBuf::from(first_line.trim());
                if path.exists() {
                    tracing::debug!("Found Ghostscript in PATH: {:?}", path);
                    return Some(path);
                }
            }
        }
    }

    tracing::debug!("Ghostscript not found in any location");
    None
}

/// Check if Ghostscript is installed (sync version for UI status)
#[cfg(target_os = "windows")]
pub fn is_ghostscript_installed() -> bool {
    find_ghostscript_path().is_some()
}

/// Check if Ghostscript is installed (non-Windows stub)
#[cfg(not(target_os = "windows"))]
pub fn is_ghostscript_installed() -> bool {
    true // Not needed on other platforms
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

/// Render PDF to PNG using Ghostscript (high quality, 600 DPI)
#[cfg(target_os = "windows")]
fn render_pdf_to_png(
    pdf_path: &str,
    gs_path: &std::path::Path,
) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    // Create temp output path for PNG
    let temp_dir = std::env::temp_dir();
    let png_path = temp_dir.join(format!("print_{}.png", uuid::Uuid::new_v4()));

    tracing::info!("Rendering PDF to PNG at 600 DPI...");
    tracing::info!("  PDF: {}", pdf_path);
    tracing::info!("  PNG: {:?}", png_path);

    let args = vec![
        "-dBATCH".to_string(),
        "-dNOPAUSE".to_string(),
        "-dNOSAFER".to_string(),
        "-sDEVICE=png16m".to_string(),      // 24-bit RGB PNG
        "-r600".to_string(),                 // 600 DPI - matches our print quality
        "-dTextAlphaBits=4".to_string(),     // Anti-aliasing for text
        "-dGraphicsAlphaBits=4".to_string(), // Anti-aliasing for graphics
        format!("-sOutputFile={}", png_path.to_string_lossy()),
        pdf_path.to_string(),
    ];

    let output = Command::new(gs_path)
        .args(&args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Ghostscript render failed: {}", stderr).into());
    }

    if !png_path.exists() {
        return Err("Ghostscript did not create PNG output".into());
    }

    tracing::info!("PNG rendered successfully");
    Ok(png_path)
}

/// Print image using Windows GDI with custom DEVMODE (includes media type!)
/// This is the key function - CreateDC accepts our DEVMODE directly
#[cfg(target_os = "windows")]
fn print_image_with_devmode(
    image_path: &std::path::Path,
    printer_name: &str,
    copies: u32,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Graphics::Gdi::{
        CreateDCW, DeleteDC, SetStretchBltMode, StretchDIBits, GetDeviceCaps,
        BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HALFTONE, SRCCOPY,
        HORZRES, VERTRES, LOGPIXELSX, LOGPIXELSY, DEVMODEW, RGBQUAD,
    };
    use windows::Win32::Graphics::Printing::{
        ClosePrinter, DocumentPropertiesW, OpenPrinterW,
    };
    use windows::Win32::Storage::Xps::{StartDocW, StartPage, EndPage, EndDoc, DOCINFOW};

    // DEVMODE flags (constants)
    const DM_OUT_BUFFER: u32 = 2;
    const DM_IN_BUFFER: u32 = 8;
    const DM_PRINTQUALITY: u32 = 0x0400;
    const DM_YRESOLUTION: u32 = 0x2000;
    const DM_MEDIATYPE: u32 = 0x08000000;
    const DM_COPIES: u32 = 0x0100;

    tracing::info!("=== PRINTING WITH CUSTOM DEVMODE ===");
    tracing::info!("Image: {:?}", image_path);
    tracing::info!("Printer: {}", printer_name);
    tracing::info!("Copies: {}", copies);

    // Load the image
    let img = image::open(image_path)?;
    let rgb_img = img.to_rgb8();
    let (width, height) = rgb_img.dimensions();
    tracing::info!("Image dimensions: {}x{} pixels", width, height);

    // Convert printer name to wide string
    let printer_name_wide: Vec<u16> = printer_name.encode_utf16().chain(std::iter::once(0)).collect();

    unsafe {
        // Step 1: Open printer
        let mut hprinter = HANDLE::default();
        let result = OpenPrinterW(
            PCWSTR(printer_name_wide.as_ptr()),
            &mut hprinter,
            None,
        );

        if result.is_err() {
            return Err(format!("Failed to open printer: {}", printer_name).into());
        }

        tracing::info!("Opened printer handle");

        // Step 2: Get DEVMODE size
        let devmode_size = DocumentPropertiesW(
            None,
            hprinter,
            PCWSTR(printer_name_wide.as_ptr()),
            None,
            None,
            0,
        );

        if devmode_size <= 0 {
            let _ = ClosePrinter(hprinter);
            return Err("Failed to get DEVMODE size".into());
        }

        tracing::info!("DEVMODE size: {} bytes", devmode_size);

        // Step 3: Allocate and get DEVMODE
        let mut devmode_buffer = vec![0u8; devmode_size as usize];
        let devmode_ptr = devmode_buffer.as_mut_ptr() as *mut DEVMODEW;

        let result = DocumentPropertiesW(
            None,
            hprinter,
            PCWSTR(printer_name_wide.as_ptr()),
            Some(devmode_ptr),
            None,
            DM_OUT_BUFFER,
        );

        if result < 0 {
            let _ = ClosePrinter(hprinter);
            return Err("Failed to get DEVMODE".into());
        }

        // Step 4: Modify DEVMODE for our settings using raw memory offsets
        // DEVMODEW (Unicode) structure offsets:
        // dmDeviceName: 0-63 (WCHAR[32] = 64 bytes)
        // dmSpecVersion: 64, dmDriverVersion: 66, dmSize: 68, dmDriverExtra: 70
        // dmFields: 72 (DWORD)
        // dmOrientation: 76, dmPaperSize: 78, dmPaperLength: 80, dmPaperWidth: 82
        // dmScale: 84, dmCopies: 86, dmDefaultSource: 88, dmPrintQuality: 90
        // dmColor: 92, dmDuplex: 94, dmYResolution: 96
        // dmFormName: 102-165 (WCHAR[32])
        // ... more fields ...
        // dmMediaType: 196 (DWORD)
        let dm_bytes = devmode_buffer.as_mut_ptr();

        // Read and modify dmFields at offset 72
        let dm_fields_ptr = dm_bytes.add(72) as *mut u32;
        let mut dm_fields = std::ptr::read_unaligned(dm_fields_ptr);
        dm_fields |= DM_PRINTQUALITY | DM_YRESOLUTION | DM_MEDIATYPE | DM_COPIES;
        std::ptr::write_unaligned(dm_fields_ptr, dm_fields);

        // Set dmCopies at offset 86
        let dm_copies_ptr = dm_bytes.add(86) as *mut i16;
        std::ptr::write_unaligned(dm_copies_ptr, copies as i16);

        // Set dmPrintQuality at offset 90
        let dm_print_quality_ptr = dm_bytes.add(90) as *mut i16;
        std::ptr::write_unaligned(dm_print_quality_ptr, 600);

        // Set dmYResolution at offset 96
        let dm_y_resolution_ptr = dm_bytes.add(96) as *mut i16;
        std::ptr::write_unaligned(dm_y_resolution_ptr, 600);

        // Set dmMediaType at offset 196 - THIS IS THE KEY SETTING!
        let dm_media_type_ptr = dm_bytes.add(196) as *mut u32;
        std::ptr::write_unaligned(dm_media_type_ptr, 258); // Premium Presentation Matte

        tracing::info!("Set DEVMODE: 600 DPI, MediaType=258 (Premium Matte), Copies={}", copies);

        // Step 5: Validate DEVMODE via DocumentProperties (merge with driver)
        let result = DocumentPropertiesW(
            None,
            hprinter,
            PCWSTR(printer_name_wide.as_ptr()),
            Some(devmode_ptr),
            Some(devmode_ptr),
            DM_IN_BUFFER | DM_OUT_BUFFER,
        );

        tracing::info!("DocumentProperties validate result: {}", result);

        // Close printer handle (we'll use CreateDC next)
        let _ = ClosePrinter(hprinter);

        // Step 6: Create printer DC with OUR DEVMODE
        // This is the key - CreateDC accepts DEVMODE parameter!
        let hdc = CreateDCW(
            PCWSTR::null(),
            PCWSTR(printer_name_wide.as_ptr()),
            PCWSTR::null(),
            Some(devmode_ptr),  // <-- THIS passes our media type 258!
        );

        if hdc.is_invalid() {
            return Err("Failed to create printer DC with DEVMODE".into());
        }

        tracing::info!("Created printer DC with custom DEVMODE");

        // Step 7: Start document
        let doc_name: Vec<u16> = "AnyMobile Label".encode_utf16().chain(std::iter::once(0)).collect();
        let doc_info = DOCINFOW {
            cbSize: std::mem::size_of::<DOCINFOW>() as i32,
            lpszDocName: PCWSTR(doc_name.as_ptr()),
            lpszOutput: PCWSTR::null(),
            lpszDatatype: PCWSTR::null(),
            fwType: 0,
        };

        let job_id = StartDocW(hdc, &doc_info);
        if job_id <= 0 {
            let _ = DeleteDC(hdc);
            return Err("Failed to start print job".into());
        }

        tracing::info!("Started print job ID: {}", job_id);

        // Step 8: Start page
        if StartPage(hdc) <= 0 {
            EndDoc(hdc);
            let _ = DeleteDC(hdc);
            return Err("Failed to start page".into());
        }

        // Step 9: Get printer page size in pixels
        let page_width = GetDeviceCaps(hdc, HORZRES);
        let page_height = GetDeviceCaps(hdc, VERTRES);
        let dpi_x = GetDeviceCaps(hdc, LOGPIXELSX);
        let dpi_y = GetDeviceCaps(hdc, LOGPIXELSY);

        tracing::info!("Printer page: {}x{} pixels at {}x{} DPI", page_width, page_height, dpi_x, dpi_y);

        // Step 10: Calculate ACTUAL SIZE print dimensions
        // Image was rendered at 600 DPI, convert to printer DPI for actual size
        let print_width = (width as i32 * dpi_x) / 600;
        let print_height = (height as i32 * dpi_y) / 600;

        // CENTER the image on the page
        let dest_x = (page_width - print_width) / 2;
        let dest_y = (page_height - print_height) / 2;

        tracing::info!("Print size: {}x{} pixels (actual size at {} DPI)", print_width, print_height, dpi_x);
        tracing::info!("Centered at: ({}, {})", dest_x, dest_y);

        // Create BITMAPINFO
        // Windows DIB is BGR, bottom-up by default
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32), // Negative = top-down
                biPlanes: 1,
                biBitCount: 24,
                biCompression: BI_RGB.0,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [RGBQUAD::default()],
        };

        // Convert RGB to BGR for Windows
        let mut bgr_data: Vec<u8> = Vec::with_capacity((width * height * 3) as usize);
        for pixel in rgb_img.pixels() {
            bgr_data.push(pixel[2]); // B
            bgr_data.push(pixel[1]); // G
            bgr_data.push(pixel[0]); // R
        }

        // Pad rows to 4-byte boundary (Windows requirement)
        let row_size = ((width * 3 + 3) / 4) * 4;
        let mut padded_data: Vec<u8> = Vec::with_capacity((row_size * height) as usize);
        for y in 0..height {
            let row_start = (y * width * 3) as usize;
            let row_end = row_start + (width * 3) as usize;
            padded_data.extend_from_slice(&bgr_data[row_start..row_end]);
            // Add padding bytes
            for _ in 0..(row_size - width * 3) {
                padded_data.push(0);
            }
        }

        // Set stretch mode for quality
        SetStretchBltMode(hdc, HALFTONE);

        // Step 11: Draw image to printer DC (centered, actual size)
        let result = StretchDIBits(
            hdc,
            dest_x,                 // dest x (centered)
            dest_y,                 // dest y (centered)
            print_width,            // dest width (actual size)
            print_height,           // dest height (actual size)
            0,                      // src x
            0,                      // src y
            width as i32,           // src width
            height as i32,          // src height
            Some(padded_data.as_ptr() as *const std::ffi::c_void),
            &bmi,
            DIB_RGB_COLORS,
            SRCCOPY,
        );

        if result == 0 {
            EndPage(hdc);
            EndDoc(hdc);
            let _ = DeleteDC(hdc);
            return Err("StretchDIBits failed".into());
        }

        tracing::info!("StretchDIBits drew {} scan lines", result);

        // Step 12: End page and document
        EndPage(hdc);
        EndDoc(hdc);
        let _ = DeleteDC(hdc);

        tracing::info!("=== PRINT JOB SENT SUCCESSFULLY ===");
    }

    Ok(())
}

/// Print PDF using Ghostscript to render + Windows GDI with custom DEVMODE
/// This approach passes our DEVMODE (with media type 258) directly to CreateDC
/// No admin rights needed - no SetPrinter call!
#[cfg(target_os = "windows")]
async fn print_pdf_ghostscript(
    pdf_path: &str,
    printer_name: Option<&str>,
    copies: u32,
    gs_path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing::info!("=== WINDOWS PRINT (GDI with Custom DEVMODE) ===");
    tracing::info!("PDF path: {}", pdf_path);
    tracing::info!("Printer: {:?}", printer_name);
    tracing::info!("Copies: {}", copies);
    tracing::info!("Ghostscript path: {:?}", gs_path);

    // Get printer name (use default if not specified)
    let printer = match printer_name {
        Some(name) => name.to_string(),
        None => {
            let output = Command::new("powershell")
                .args(["-Command", "(Get-WmiObject -Query \"SELECT * FROM Win32_Printer WHERE Default=$true\").Name"])
                .creation_flags(CREATE_NO_WINDOW)
                .output()?;
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
    };

    tracing::info!("Using printer: {}", printer);

    // Step 1: Render PDF to high-quality PNG using Ghostscript
    tracing::info!("Step 1: Rendering PDF to PNG at 600 DPI...");
    let png_path = render_pdf_to_png(pdf_path, gs_path)?;

    // Step 2: Print PNG using Windows GDI with our DEVMODE
    // This is the key - CreateDC accepts our DEVMODE with media type 258!
    tracing::info!("Step 2: Printing PNG with custom DEVMODE (media type 258)...");
    let result = print_image_with_devmode(&png_path, &printer, copies);

    // Clean up temp PNG
    if let Err(e) = std::fs::remove_file(&png_path) {
        tracing::warn!("Failed to clean up temp PNG: {}", e);
    }

    result?;

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
// macOS/Linux Implementation (CUPS)
// ============================================================================

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn list_printers_unix() -> Result<Vec<PrinterInfo>, Box<dyn std::error::Error>> {
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

#[cfg(any(target_os = "macos", target_os = "linux"))]
async fn print_pdf_unix(
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
            // Epson-specific: 1200 DPI, highest quality, premium matte paper
            // ET-3830 supports up to 5760×1440 DPI, so 1200 is well within range
            args.extend([
                "-o".to_string(), "Resolution=1200x1200dpi".to_string(),
                "-o".to_string(), "EPIJ_Qual=307".to_string(),
                "-o".to_string(), "EPIJ_Medi=12".to_string(),  // Premium Presentation Paper Matte
            ]);
            tracing::info!("Applying Epson high-quality settings: 1200dpi, quality=307, matte paper");
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

    // Diagnostic logging for print quality debugging
    if let Ok(metadata) = std::fs::metadata(pdf_path) {
        tracing::info!("PDF file size: {} bytes ({:.2} KB)", metadata.len(), metadata.len() as f64 / 1024.0);
    }
    tracing::info!("=== LINUX/macOS PRINT (CUPS lp) ===");
    tracing::info!("Full lp command: lp {}", args.join(" "));
    tracing::info!("Executing lp with args: {:?}", args);

    let status = Command::new("lp")
        .args(&args)
        .status()?;

    if !status.success() {
        tracing::error!("lp print command failed with status: {:?}", status);
        return Err("lp print command failed".into());
    }

    tracing::info!("=== LINUX/macOS PRINT COMPLETE ===");
    Ok(())
}
