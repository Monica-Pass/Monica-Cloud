/**
 * Registers the app's Material Symbols as inline SVG with m3e-icon (see scripts/icons.ts).
 * Registered icons render immediately and the browser never downloads the ~4 MB icon font;
 * any name missing here still renders through the font as before.
 */

import { registerIcon } from "@m3e/web/icon";
import { iconPaths } from "./icons.generated";

const viewBox = "0 -960 960 960";

export function registerIcons() {
  for (const [name, [outlined, filled]] of Object.entries(iconPaths)) {
    registerIcon(name, "outlined", { outlined: { viewBox, path: outlined }, filled: { viewBox, path: filled ?? outlined } });
  }
}
