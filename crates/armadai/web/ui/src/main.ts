import { mount } from "svelte";
import App from "./App.svelte";
// Self-hosted IBM Plex (OFL) via @fontsource — Vite bundles the woff2 into
// dist/assets and registers the @font-face; --font-ui/--font-mono reference
// these families. UI = Sans, telemetry/data = Mono; weights 400 + 600.
import "@fontsource/ibm-plex-sans/400.css";
import "@fontsource/ibm-plex-sans/600.css";
import "@fontsource/ibm-plex-mono/400.css";
import "@fontsource/ibm-plex-mono/600.css";
import "./tokens.css";

const app = mount(App, { target: document.getElementById("app")! });
export default app;
