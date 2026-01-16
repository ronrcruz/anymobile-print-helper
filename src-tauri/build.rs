fn main() {
    let mut builder = tauri_build::Build::new();

    #[cfg(windows)]
    {
        // Custom manifest requesting administrator privileges for SetPrinter API
        let manifest = r#"
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="requireAdministrator" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>
"#;
        builder = builder.windows(
            tauri_build::WindowsAttributes::new().app_manifest(manifest)
        );
    }

    builder.run();
}
