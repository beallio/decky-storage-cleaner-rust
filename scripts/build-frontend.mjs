// Bundles src/index.tsx into dist/index.js the way Decky Loader loads it: one ES module with a
// default export. React, react-dom and @decky/ui are Steam's own copies (window globals), and
// @decky/manifest is plugin.json. Pass --watch to rebuild on change.
import { build, context } from "esbuild";
import { readFileSync } from "node:fs";

const manifest = readFileSync("plugin.json", "utf8");
const { version } = JSON.parse(readFileSync("package.json", "utf8"));
const globals = { react: "SP_REACT", "react-dom": "SP_REACTDOM", "@decky/ui": "DFL" };

const options = {
  entryPoints: ["src/index.tsx"],
  outfile: "dist/index.js",
  bundle: true,
  format: "esm",
  target: "es2020",
  sourcemap: true,
  define: { "process.env.NODE_ENV": '"production"', __PLUGIN_VERSION__: JSON.stringify(version) },
  logLevel: "warning",
  plugins: [
    {
      name: "decky-globals",
      setup(b) {
        b.onResolve({ filter: /^(react|react-dom|@decky\/ui|@decky\/manifest)$/ }, ({ path }) => ({ path, namespace: "decky" }));
        b.onLoad({ filter: /.*/, namespace: "decky" }, ({ path }) =>
          path === "@decky/manifest"
            ? { contents: manifest, loader: "json" }
            : { contents: `module.exports = window.${globals[path]};`, loader: "js" }
        );
      }
    }
  ]
};

if (process.argv.includes("--watch")) {
  await (await context(options)).watch();
} else {
  await build(options);
}
