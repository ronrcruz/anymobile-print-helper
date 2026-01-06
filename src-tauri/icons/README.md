# Icons

This folder should contain the following icon files for the application:

- `32x32.png` - 32x32 pixel PNG icon
- `128x128.png` - 128x128 pixel PNG icon
- `128x128@2x.png` - 256x256 pixel PNG icon (for retina displays)
- `icon.icns` - macOS icon bundle
- `icon.ico` - Windows icon file
- `icon.png` - Base icon for system tray

## Generating Icons

You can use [Tauri's icon generator](https://tauri.app/v1/guides/features/icons/):

```bash
npm run tauri icon /path/to/your/1024x1024-icon.png
```

Or use online tools like:
- https://icon.kitchen/
- https://realfavicongenerator.net/

## Suggested Design

For AnyMobile Print Helper, consider:
- Printer emoji or icon in AnyMobile brand colors
- Navy blue (#1B4F8C) background
- White printer icon
