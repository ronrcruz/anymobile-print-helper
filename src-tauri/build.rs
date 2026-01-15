fn main() {
    // Embed Windows manifest requesting administrator privileges
    #[cfg(windows)]
    {
        use embed_manifest::{embed_manifest, new_manifest, manifest::ExecutionLevel};

        embed_manifest(
            new_manifest("AnyMobile Print Helper")
                .requested_execution_level(ExecutionLevel::RequireAdministrator)
        ).expect("Failed to embed Windows manifest");
    }

    tauri_build::build()
}
