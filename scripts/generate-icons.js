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
import zlib from 'zlib';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

/**
 * Create a valid PNG file with a solid color
 * Navy blue: #1E3A5F
 */
function createPNG(width, height) {
  // PNG Signature
  const signature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);

  // IHDR chunk
  const ihdrData = Buffer.alloc(13);
  ihdrData.writeUInt32BE(width, 0);   // Width
  ihdrData.writeUInt32BE(height, 4);  // Height
  ihdrData.writeUInt8(8, 8);          // Bit depth
  ihdrData.writeUInt8(2, 9);          // Color type (2 = RGB)
  ihdrData.writeUInt8(0, 10);         // Compression method
  ihdrData.writeUInt8(0, 11);         // Filter method
  ihdrData.writeUInt8(0, 12);         // Interlace method

  const ihdr = createChunk('IHDR', ihdrData);

  // Create raw image data (filter byte + RGB for each row)
  const rawData = Buffer.alloc(height * (1 + width * 3));

  // Navy blue color: #1E3A5F
  const r = 0x1E;
  const g = 0x3A;
  const b = 0x5F;

  for (let y = 0; y < height; y++) {
    const rowStart = y * (1 + width * 3);
    rawData[rowStart] = 0; // Filter: None
    for (let x = 0; x < width; x++) {
      const pixelStart = rowStart + 1 + x * 3;
      rawData[pixelStart] = r;
      rawData[pixelStart + 1] = g;
      rawData[pixelStart + 2] = b;
    }
  }

  // Compress with zlib
  const compressed = zlib.deflateSync(rawData);
  const idat = createChunk('IDAT', compressed);

  // IEND chunk
  const iend = createChunk('IEND', Buffer.alloc(0));

  return Buffer.concat([signature, ihdr, idat, iend]);
}

/**
 * Create a PNG chunk with type and data
 */
function createChunk(type, data) {
  const typeBuffer = Buffer.from(type);
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length, 0);

  const crcData = Buffer.concat([typeBuffer, data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(crcData), 0);

  return Buffer.concat([length, typeBuffer, data, crc]);
}

/**
 * CRC32 calculation for PNG
 */
function crc32(data) {
  let crc = 0xFFFFFFFF;
  const table = getCRC32Table();

  for (let i = 0; i < data.length; i++) {
    crc = (crc >>> 8) ^ table[(crc ^ data[i]) & 0xFF];
  }

  return (crc ^ 0xFFFFFFFF) >>> 0;
}

let crcTable = null;
function getCRC32Table() {
  if (crcTable) return crcTable;

  crcTable = new Uint32Array(256);
  for (let i = 0; i < 256; i++) {
    let c = i;
    for (let j = 0; j < 8; j++) {
      c = (c & 1) ? (0xEDB88320 ^ (c >>> 1)) : (c >>> 1);
    }
    crcTable[i] = c;
  }
  return crcTable;
}

// Create icons directory
const iconsDir = path.join(__dirname, '..', 'src-tauri', 'icons');
if (!fs.existsSync(iconsDir)) {
  fs.mkdirSync(iconsDir, { recursive: true });
}

// Generate icons at different sizes
const sizes = {
  '32x32.png': 32,
  '128x128.png': 128,
  '128x128@2x.png': 256,
  'icon.png': 512,
};

for (const [filename, size] of Object.entries(sizes)) {
  const png = createPNG(size, size);
  fs.writeFileSync(path.join(iconsDir, filename), png);
  console.log(`Created ${filename} (${size}x${size})`);
}

console.log('\n Placeholder icons created in src-tauri/icons/');
console.log('');
console.log('For production, create proper branded icons:');
console.log('   1. Create a 1024x1024 PNG icon');
console.log('   2. Run: npx tauri icon /path/to/icon.png');
console.log('   Or use: https://icon.kitchen/');
