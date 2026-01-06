# AnyMobile Print Helper

Desktop print helper for AnyMobile label printing with correct settings.

## Features

- **One-click printing** from the AnyMobile web app
- **No scaling** - labels print at actual size (prevents misalignment)
- **Automatic settings** - no more adjusting Windows print dialog
- **System tray** - runs quietly in the background
- **Auto-start** - launches on system startup

## How It Works

1. The helper runs a local HTTP server on `localhost:9847`
2. When you click "Print Now" in the AnyMobile staff portal, the web app sends the PDF to the helper
3. The helper prints using SumatraPDF (Windows) or `lp` (macOS) with correct settings

## Requirements

### Windows
- Windows 10 or later
- [SumatraPDF](https://www.sumatrapdfreader.org/) (optional, for best results)

### macOS
- macOS 10.13 or later

## Development

### Prerequisites

- [Rust](https://rustup.rs/) (latest stable)
- [Node.js](https://nodejs.org/) (18+)
- [Tauri CLI](https://tauri.app/)

### Setup

```bash
# Install dependencies
npm install

# Run in development mode
npm run dev

# Build for production
npm run build
```

### Building for Release

```bash
# Windows
npm run tauri build -- --target x86_64-pc-windows-msvc

# macOS
npm run tauri build -- --target x86_64-apple-darwin
npm run tauri build -- --target aarch64-apple-darwin
```

## API Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/ping` | GET | Health check, returns version and printer list |
| `/printers` | GET | List available printers |
| `/print` | POST | Print a PDF (multipart form with `pdf` field) |

## License

MIT
