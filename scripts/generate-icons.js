#!/usr/bin/env node
/**
 * Generate placeholder icons for Tauri build
 * Run: node scripts/generate-icons.js
 *
 * For production, replace with proper branded icons using:
 * - https://icon.kitchen/
 * - npx tauri icon /path/to/1024x1024.png
 */

import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

// Simple PNG generator - creates a solid color square with a printer emoji overlay
// This is a minimal 32x32 navy blue PNG
const createPNG = (size) => {
  // PNG header + IHDR + IDAT + IEND for a solid navy blue square
  // Navy blue: #1B4F8C = RGB(27, 79, 140)

  const width = size;
  const height = size;

  // Create raw pixel data (RGBA)
  const pixels = Buffer.alloc(width * height * 4);
  for (let i = 0; i < width * height; i++) {
    pixels[i * 4] = 27;      // R
    pixels[i * 4 + 1] = 79;  // G
    pixels[i * 4 + 2] = 140; // B
    pixels[i * 4 + 3] = 255; // A
  }

  // For a proper PNG, we'd need zlib compression
  // Instead, let's create an uncompressed BMP-style approach
  // Actually, let's just use a pre-made tiny PNG as base64

  return null; // We'll use a different approach
};

// Base64 encoded 32x32 navy blue PNG with printer icon
// Generated externally - this is a simple placeholder
const ICON_32_BASE64 = `iVBORw0KGgoAAAANSUhEUgAAACAAAAAgCAYAAABzenr0AAAABHNCSVQICAgIfAhkiAAAAAlwSFlzAAAA7AAAAOwBeShxvQAAABl0RVh0U29mdHdhcmUAd3d3Lmlua3NjYXBlLm9yZ5vuPBoAAAGRSURBVFiF7ZY9TsNAEIW/WYcQKQVSBBIlNXdIR0VHTUdFxQm4ATUNHCEHoKCgpqFEogAhkRISOz8UdhKvvevYJqLgSSvtzrx5+2d2DH9dsusPuBZwG/AGqADrwEfg0cxKf0UAuAbKZlb6d+Cfys/6C/DqH/gnCRwBu8AlMAmsAQe9BshLkOT8DXAJjAMV4JVIoK8Au4EYKYFaFGimgXvg2cxWRSQCHIF9YBbYI+7WVdcB2osI8fefqvvAGLAEfAQWzexN9e4VYG8BroChgARKJuRE5AJYDJGhBSwH0nkOLKoqqOsygWRFxBXwIiIVYBn4AmZV1e0r4FxEjoF5oA48AmVVbaSBBCKyA6yparNvAFVtAGPABVA1s3ZgQVXPgR0zexeR8qCJkwEwbWbvfdYUkXngSUQWgMdBAyQPFJENoGZmZ0A1cGsF2BaRcaCZBJKBbZjZvqrey/b1RERCYFtV68BSJoiiKLqZ1y50RysBnKjqXeCxWpIISNQ5hnkn6TvA0r/uM5d/PXj5N/vPg5c/Lb9h0PkNaKuX2AAAAABJRU5ErkJggg==`;

const ICON_128_BASE64 = ICON_32_BASE64; // Use same for now, will be scaled

// Create icons directory structure
const iconsDir = path.join(__dirname, '..', 'src-tauri', 'icons');

// Ensure directory exists
if (!fs.existsSync(iconsDir)) {
  fs.mkdirSync(iconsDir, { recursive: true });
}

// Write PNG files
const iconBuffer = Buffer.from(ICON_32_BASE64, 'base64');

fs.writeFileSync(path.join(iconsDir, '32x32.png'), iconBuffer);
fs.writeFileSync(path.join(iconsDir, '128x128.png'), iconBuffer);
fs.writeFileSync(path.join(iconsDir, '128x128@2x.png'), iconBuffer);
fs.writeFileSync(path.join(iconsDir, 'icon.png'), iconBuffer);

console.log('✅ Placeholder icons created in src-tauri/icons/');
console.log('');
console.log('⚠️  These are placeholder icons. For production, create proper branded icons:');
console.log('   1. Create a 1024x1024 PNG icon');
console.log('   2. Run: npx tauri icon /path/to/icon.png');
console.log('   Or use: https://icon.kitchen/');
