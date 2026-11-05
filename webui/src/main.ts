import "@fontsource-variable/roboto";
// Fallback only: icons registered by registerIcons() render as SVG, so this font is
// downloaded only if a name is missing from src/lib/icons.generated.ts.
import "material-symbols/outlined.css";
import { createApp } from "vue";
import App from "./App.vue";
import { registerIcons } from "./lib/icons";
import "./styles.css";

registerIcons();
createApp(App).mount("#root");
