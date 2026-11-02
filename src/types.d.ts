// package.json version, injected by scripts/build-frontend.mjs.
declare const __PLUGIN_VERSION__: string;

declare module "*.svg" {
  const content: string;
  export default content;
}

declare module "*.png" {
  const content: string;
  export default content;
}

declare module "*.jpg" {
  const content: string;
  export default content;
}

declare module "*.json" {
  const content: string;
  export default content;
}
