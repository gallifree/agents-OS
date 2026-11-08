import type { Config } from "vike/types";
import vikeReact from "vike-react/config";

// https://vike.dev/config
const config: Config = {
  title: "rust-agents",
  description: "Self-hosted AI agent platform",
  // SPA: the Rust backend serves the static build, no Node server at runtime.
  ssr: false,
  prerender: true,
  extends: [vikeReact],
};

export default config;
