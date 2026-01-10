//! Certificate management for Windows
//! Handles checking if cert is trusted and installing to Windows stores

use std::path::PathBuf;

#[cfg(target_os = "windows")]
use std::process::Command;

/// Get the path to the certificate directory
pub fn get_cert_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("anymobile-print-helper")
        .join("certs")
}

/// Get the path to the localhost certificate
pub fn get_cert_path() -> PathBuf {
    get_cert_dir().join("localhost.crt")
}

/// Check if the localhost certificate is installed in the Windows trusted root store
#[cfg(target_os = "windows")]
pub fn is_cert_trusted() -> Result<bool, String> {
    let ps_script = r#"
$certs = Get-ChildItem -Path Cert:\CurrentUser\Root | Where-Object { $_.Subject -like "*localhost*" }
if ($certs) { "true" } else { "false" }
"#;

    let output = Command::new("powershell")
        .args(["-ExecutionPolicy", "Bypass", "-NoProfile", "-Command", ps_script])
        .output()
        .map_err(|e| format!("Failed to run PowerShell: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_lowercase();
    tracing::debug!("Certificate trust check result: {}", stdout);
    Ok(stdout == "true")
}

/// Install certificate to CurrentUser trusted root store (no admin required)
#[cfg(target_os = "windows")]
pub fn install_cert_current_user() -> Result<(), String> {
    let cert_path = get_cert_path();

    if !cert_path.exists() {
        return Err("Certificate not found. Please restart the application.".to_string());
    }

    let cert_path_str = cert_path.to_string_lossy();

    let ps_script = format!(r#"
$ErrorActionPreference = 'Stop'
try {{
    $certPath = '{}'

    # Read the PEM file
    $pemContent = Get-Content $certPath -Raw

    # Extract base64 content (remove headers and whitespace)
    $base64 = $pemContent -replace '-----BEGIN CERTIFICATE-----', '' `
                          -replace '-----END CERTIFICATE-----', '' `
                          -replace '\s', ''

    # Convert to bytes
    $certBytes = [Convert]::FromBase64String($base64)

    # Create certificate object
    $cert = [System.Security.Cryptography.X509Certificates.X509Certificate2]::new($certBytes)

    # Open the CurrentUser Root store
    $store = New-Object System.Security.Cryptography.X509Certificates.X509Store("Root", "CurrentUser")
    $store.Open("ReadWrite")

    # Add the certificate
    $store.Add($cert)
    $store.Close()

    Write-Host "SUCCESS"
    exit 0
}} catch {{
    Write-Host "ERROR: $_"
    exit 1
}}
"#, cert_path_str);

    tracing::info!("Installing certificate to CurrentUser\\Root store");

    let output = Command::new("powershell")
        .args(["-ExecutionPolicy", "Bypass", "-NoProfile", "-Command", &ps_script])
        .output()
        .map_err(|e| format!("Failed to run PowerShell: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    tracing::debug!("Install stdout: {}", stdout);
    tracing::debug!("Install stderr: {}", stderr);

    if output.status.success() && stdout.contains("SUCCESS") {
        tracing::info!("Certificate installed successfully to CurrentUser store");
        Ok(())
    } else {
        let error_msg = if stderr.is_empty() { stdout.to_string() } else { stderr.to_string() };
        tracing::error!("Certificate installation failed: {}", error_msg);
        Err(format!("Installation failed: {}", error_msg))
    }
}

/// Install certificate to LocalMachine store using elevated PowerShell (requires UAC)
#[cfg(target_os = "windows")]
pub fn install_cert_local_machine() -> Result<(), String> {
    let cert_path = get_cert_path();

    if !cert_path.exists() {
        return Err("Certificate not found. Please restart the application.".to_string());
    }

    let cert_path_str = cert_path.to_string_lossy();

    // Create a temporary script file for elevation
    let script_content = format!(r#"
Add-Type -AssemblyName System.Windows.Forms
$ErrorActionPreference = 'Stop'
try {{
    $certPath = '{}'
    $pemContent = Get-Content $certPath -Raw
    $base64 = $pemContent -replace '-----BEGIN CERTIFICATE-----', '' `
                          -replace '-----END CERTIFICATE-----', '' `
                          -replace '\s', ''
    $certBytes = [Convert]::FromBase64String($base64)
    $cert = [System.Security.Cryptography.X509Certificates.X509Certificate2]::new($certBytes)
    $store = New-Object System.Security.Cryptography.X509Certificates.X509Store("Root", "LocalMachine")
    $store.Open("ReadWrite")
    $store.Add($cert)
    $store.Close()
    [System.Windows.Forms.MessageBox]::Show("Certificate installed successfully! Please restart your browser.", "AnyMobile Print Helper", "OK", "Information")
}} catch {{
    [System.Windows.Forms.MessageBox]::Show("Installation failed: $_", "Error", "OK", "Error")
}}
"#, cert_path_str);

    // Write to temp file
    let temp_dir = std::env::temp_dir();
    let script_path = temp_dir.join("install_cert.ps1");
    std::fs::write(&script_path, script_content)
        .map_err(|e| format!("Failed to write script: {}", e))?;

    tracing::info!("Running elevated certificate installation");

    // Run with elevation
    let output = Command::new("powershell")
        .args([
            "-Command",
            &format!(
                "Start-Process powershell -Verb RunAs -ArgumentList '-ExecutionPolicy Bypass -NoProfile -File \"{}\"' -Wait",
                script_path.to_string_lossy()
            )
        ])
        .output()
        .map_err(|e| format!("Failed to run elevated PowerShell: {}", e))?;

    // Clean up temp file
    let _ = std::fs::remove_file(&script_path);

    if output.status.success() {
        tracing::info!("Elevated certificate installation completed");
        Ok(())
    } else {
        Err("User cancelled or installation failed".to_string())
    }
}

/// Remove certificate from Windows trusted stores
#[cfg(target_os = "windows")]
pub fn remove_cert_from_store() -> Result<(), String> {
    let ps_script = r#"
$ErrorActionPreference = 'Stop'
try {
    # Remove from CurrentUser
    $certs = Get-ChildItem -Path Cert:\CurrentUser\Root | Where-Object { $_.Subject -like "*localhost*" }
    foreach ($cert in $certs) {
        $store = New-Object System.Security.Cryptography.X509Certificates.X509Store("Root", "CurrentUser")
        $store.Open("ReadWrite")
        $store.Remove($cert)
        $store.Close()
    }
    Write-Host "SUCCESS"
} catch {
    Write-Host "ERROR: $_"
}
"#;

    let output = Command::new("powershell")
        .args(["-ExecutionPolicy", "Bypass", "-NoProfile", "-Command", ps_script])
        .output()
        .map_err(|e| format!("Failed to run PowerShell: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    if stdout.contains("SUCCESS") {
        Ok(())
    } else {
        Err(format!("Failed to remove certificate: {}", stdout))
    }
}

// Non-Windows stubs
#[cfg(not(target_os = "windows"))]
pub fn is_cert_trusted() -> Result<bool, String> {
    // On macOS/Linux, we don't need to install the cert to a store
    // The browser will prompt the user to accept it
    Ok(true)
}

#[cfg(not(target_os = "windows"))]
pub fn install_cert_current_user() -> Result<(), String> {
    Err("Certificate store installation is only available on Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn install_cert_local_machine() -> Result<(), String> {
    Err("Certificate store installation is only available on Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn remove_cert_from_store() -> Result<(), String> {
    Err("Certificate store management is only available on Windows".to_string())
}
