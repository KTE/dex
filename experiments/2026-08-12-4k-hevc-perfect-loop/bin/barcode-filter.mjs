#!/usr/bin/env node
// @ts-check
// Prints an ffmpeg filter chain (or the strip height) for a given frame height.
// Exists so shell scripts consume the single geometry definition in
// lib/barcode.mjs instead of re-deriving it — a burner and decoder that drifted
// apart would fail silently and be miserable to diagnose.
import { burnFilter, decodeFilter, barHeight } from '../lib/barcode.mjs';

const [mode, heightArg] = process.argv.slice(2);
const height = Number(heightArg);
if (!Number.isFinite(height) || height <= 0) {
  console.error('usage: barcode-filter.mjs <burn|decode|height> <frame-height>');
  process.exit(2);
}
if (mode === 'burn') process.stdout.write(burnFilter(height));
else if (mode === 'decode') process.stdout.write(decodeFilter(height));
else if (mode === 'height') process.stdout.write(String(barHeight(height)));
else {
  console.error(`unknown mode: ${mode}`);
  process.exit(2);
}
