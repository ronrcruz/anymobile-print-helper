fn main() {
    // Embed Windows manifest requesting administrator privileges
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_manifest_file("app.manifest");
        if let Err(e) = res.compile() {
            eprintln!("Warning: Failed to compile Windows resources: {}", e);
        }
    }

    tauri_build::build()
}
