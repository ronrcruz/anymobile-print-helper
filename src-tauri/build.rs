fn main() {
    // No admin manifest needed - we use CreateDC with DEVMODE for per-job settings
    // instead of SetPrinter which requires admin rights
    tauri_build::build();
}
