/**
 * Writes .br and .gz next to every compressible file in dist/ after `vite build`.
 * The server serves them to browsers that accept the encoding (tower-http ServeDir
 * precompressed), so assets are compressed once at build time instead of per request.
 *
 *   node scripts/compress.ts
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import zlib from "node:zlib";

const dist = fileURLToPath(new URL("../dist", import.meta.url));
const compressible = /\.(js|mjs|css|html|svg|json|txt|xml|wasm|map)$/;

function files(dir: string): string[] {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    return entry.isDirectory() ? files(file) : [file];
  });
}

let before = 0;
let after = 0;
for (const file of files(dist)) {
  if (!compressible.test(file)) continue;
  const data = fs.readFileSync(file);
  if (data.length < 1024) continue;
  const br = zlib.brotliCompressSync(data, {
    params: { [zlib.constants.BROTLI_PARAM_QUALITY]: 11, [zlib.constants.BROTLI_PARAM_SIZE_HINT]: data.length }
  });
  const gz = zlib.gzipSync(data, { level: 9 });
  // Keep a variant only when it actually saves bytes; ServeDir falls back to the original.
  if (br.length < data.length) fs.writeFileSync(`${file}.br`, br);
  if (gz.length < data.length) fs.writeFileSync(`${file}.gz`, gz);
  before += data.length;
  after += Math.min(br.length, data.length);
}
console.log(`compress: ${(before / 1024).toFixed(0)} KiB of text assets → ${(after / 1024).toFixed(0)} KiB brotli`);
