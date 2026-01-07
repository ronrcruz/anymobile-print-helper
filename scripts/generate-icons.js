#!/usr/bin/env node
/**
 * Generate placeholder icons for Tauri build
 * Downloads simple solid-color PNGs from placehold.co
 */

import { execSync } from 'child_process';
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const iconsDir = path.join(__dirname, '..', 'src-tauri', 'icons');

if (!fs.existsSync(iconsDir)) {
  fs.mkdirSync(iconsDir, { recursive: true });
}

const COLOR = '1E3A5F'; // Navy blue

const icons = [
  { file: '32x32.png', size: 32 },
  { file: '128x128.png', size: 128 },
  { file: '128x128@2x.png', size: 256 },
  { file: 'icon.png', size: 512 },
];

for (const { file, size } of icons) {
  const url = `https://placehold.co/${size}x${size}/${COLOR}/${COLOR}.png`;
  const dest = path.join(iconsDir, file);
  execSync(`curl -sL "${url}" -o "${dest}"`);
  console.log(`Created ${file} (${size}x${size})`);
}

console.log('\nPlaceholder icons created.');
console.log('For production, replace with branded icons using: npx tauri icon /path/to/icon.png');
