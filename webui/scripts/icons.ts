/**
 * Generates src/lib/icons.generated.ts: SVG paths for every Material Symbols icon the WebUI
 * references, cut out of the bundled material-symbols font at exactly the axes m3e-icon draws
 * (Outlined, FILL 0 and 1, wght 400, GRAD 0, opsz 24).
 *
 * m3e-icon renders registered names as inline SVG, so browsers never download the ~4 MB
 * variable icon font. A name that is not registered still falls back to the font.
 *
 *   node scripts/icons.ts           # regenerate after adding or removing an icon
 *   node scripts/icons.ts --check   # exit 1 when the generated file is out of date (run by `npm run build`)
 *
 * Icon names are found by scanning src/ for quoted words that the font turns into a single
 * ligature glyph, which covers `name="…"`, `:name="a ? 'x' : 'y'"` and `icon: "…"` alike.
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import * as fontkit from "fontkit";
import wawoff2 from "wawoff2";

const root = fileURLToPath(new URL("..", import.meta.url));
const output = path.join(root, "src/lib/icons.generated.ts");
const fontFile = path.join(root, "node_modules/material-symbols/material-symbols-outlined.woff2");
const fontVersion = JSON.parse(fs.readFileSync(path.join(root, "node_modules/material-symbols/package.json"), "utf8")).version;

type Command = { command: string; args: number[] };
type Font = {
  layout(text: string): { glyphs: { id: number }[] };
  getVariation(settings: Record<string, number>): Font;
  getGlyph(id: number): { path: { scale(x: number, y: number): { commands: Command[] } } };
};

function sourceFiles(dir: string): string[] {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) return entry.name === "i18n" ? [] : sourceFiles(file);
    return /\.(vue|ts)$/.test(entry.name) && file !== output ? [file] : [];
  });
}

const quoted = /["'`]([a-z0-9_]{2,})["'`]/g;

/**
 * Quoted words in places that hold icon names: `<m3e-icon name="…">` and `:name="…"`,
 * `icon: "…"` properties, and declarations whose name contains "icon" (maps such as
 * `const icons = { … }` or `const emptyIcon = computed(…)`).
 */
function candidates(): string[] {
  const words = new Set<string>();
  const collect = (text: string) => {
    for (const match of text.matchAll(quoted)) words.add(match[1]);
  };
  for (const file of sourceFiles(path.join(root, "src"))) {
    const source = fs.readFileSync(file, "utf8");
    for (const [tag] of source.matchAll(/<m3e-icon\b[^>]*>/g)) {
      for (const match of tag.matchAll(/(?:^|\s)(:?)name="([^"]*)"/g)) {
        if (match[1]) collect(match[2]);
        else words.add(match[2]);
      }
    }
    for (const match of source.matchAll(/\bicon\s*:\s*["'`]([a-z0-9_]+)["'`]/g)) words.add(match[1]);
    for (const [statement] of source.matchAll(/\b(?:const|let|var)\s+\w*[iI]cons?\w*\b[^;]*;/g)) collect(statement);
  }
  return [...words].sort();
}

/** Compact relative path data in the font's 960-unit grid (y flipped to SVG orientation). */
function pathData(commands: Command[]): string {
  // Split into closed contours with absolute integer coordinates.
  const contours: { command: string; points: number[] }[][] = [];
  let current: { command: string; points: number[] }[] = [];
  for (const { command, args } of commands) {
    const points = args.map((v) => Math.round(v) || 0);
    if (command === "moveTo") {
      if (current.length) contours.push(current);
      current = [{ command, points }];
    } else if (command === "closePath") {
      if (current.length) contours.push(current);
      current = [];
    } else current.push({ command, points });
  }
  if (current.length) contours.push(current);

  let out = "";
  const put = (token: string) => {
    out += /^[a-zA-Z]/.test(token) || /[a-zA-Z]$/.test(out) || token.startsWith("-") || !out ? token : ` ${token}`;
  };
  // Everything is relative. A leading "m" counts as absolute, and after "z" the current
  // point is the start of the contour just closed.
  let x = 0;
  let y = 0;
  let last = "";
  const emit = (letter: string, values: number[]) => {
    if (letter !== last || letter === "m") put(letter);
    for (const value of values) put(String(value));
    last = letter === "m" ? "l" : letter; // coordinate pairs after a moveto are implicit linetos
  };
  for (const contour of contours) {
    const [mx, my] = contour[0].points;
    // Skip zero-area contours the font keeps only as interpolation anchors.
    if (contour.every(({ points }) => points.every((v, i) => v === (i % 2 ? my : mx)))) continue;
    emit("m", [mx - x, my - y]);
    x = mx;
    y = my;
    for (const { command, points } of contour.slice(1)) {
      const rel = points.map((v, i) => v - (i % 2 ? y : x));
      if (rel.every((v) => v === 0)) continue; // zero-length segment (a corner collapsed at this instance)
      if (command === "lineTo") {
        if (rel[1] === 0) emit("h", [rel[0]]);
        else if (rel[0] === 0) emit("v", [rel[1]]);
        else emit("l", rel);
      } else if (command === "quadraticCurveTo") emit("q", rel);
      else if (command === "bezierCurveTo") emit("c", rel);
      else throw new Error(`unsupported path command ${command}`);
      x = points[points.length - 2];
      y = points[points.length - 1];
    }
    put("z");
    last = "z";
    x = mx;
    y = my;
  }
  return out;
}

async function generate(): Promise<{ text: string; names: string[] }> {
  const ttf = Buffer.from(await wawoff2.decompress(fs.readFileSync(fontFile)));
  const font = fontkit.create(ttf) as unknown as Font;
  const axes = { wght: 400, GRAD: 0, opsz: 24 };
  const outlinedFont = font.getVariation({ ...axes, FILL: 0 });
  const filledFont = font.getVariation({ ...axes, FILL: 1 });

  const entries: string[] = [];
  const names: string[] = [];
  for (const name of candidates()) {
    const glyphs = font.layout(name).glyphs;
    if (glyphs.length !== 1) continue; // not a ligature, so not an icon name
    const outlined = pathData(outlinedFont.getGlyph(glyphs[0].id).path.scale(1, -1).commands);
    const filled = pathData(filledFont.getGlyph(glyphs[0].id).path.scale(1, -1).commands);
    if (!outlined) continue;
    names.push(name);
    entries.push(`  ${JSON.stringify(name)}: ${JSON.stringify(filled === outlined ? [outlined] : [outlined, filled])},`);
  }
  const text = [
    `// Generated by scripts/icons.ts from material-symbols ${fontVersion} (Outlined, wght 400, GRAD 0, opsz 24).`,
    "// Do not edit: run `npm run icons` after adding or removing an icon name in src/.",
    "// Each entry is [outlined path, filled path]; the filled path is omitted when both are equal.",
    "export const iconPaths: Record<string, readonly [string, string?]> = {",
    ...entries,
    "};",
    ""
  ].join("\n");
  return { text, names };
}

const { text, names } = await generate();
if (process.argv.includes("--check")) {
  const existing = fs.existsSync(output) ? fs.readFileSync(output, "utf8") : "";
  const generated = new Set([...existing.matchAll(/^ {2}"([a-z0-9_]+)":/gm)].map((m) => m[1]));
  const missing = names.filter((name) => !generated.has(name));
  if (missing.length) {
    // Missing icons still render through the font, but that costs every visitor a 4 MB download.
    console.error(`icons: not in src/lib/icons.generated.ts: ${missing.join(", ")}`);
    console.error("Run `npm run icons` and commit the result.");
    process.exit(1);
  }
  if (existing !== text) console.warn("icons: src/lib/icons.generated.ts has unused entries; `npm run icons` would drop them.");
  else console.log(`icons: ${names.length} up to date`);
} else {
  fs.writeFileSync(output, text);
  console.log(`icons: wrote ${names.length} icons (${(Buffer.byteLength(text) / 1024).toFixed(1)} KiB) to src/lib/icons.generated.ts`);
}
