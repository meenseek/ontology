import type { Plugin } from "vite";

// These ESM distributions embed CSS with style-inject. Move their original CSS
// through Vite's stylesheet pipeline so production keeps style-src 'self'.
export function graphStyles(): Plugin {
  const packages = ["float-tooltip", "three-render-objects", "3d-force-graph"];
  const prefix = "virtual:ontology-graph-style/";
  const styles = new Map<string, string>();
  return {
    name: "ontology-static-graph-styles",
    apply: "build",
    resolveId(id) { if (id.startsWith(prefix)) return "\0" + id; },
    load(id) {
      if (!id.startsWith("\0" + prefix)) return;
      const css = styles.get(id.slice(1));
      if (css === undefined) this.error(`Missing extracted graph CSS: ${id}`);
      return css;
    },
    transform(code, id) {
      const path = id.split("?")[0].replaceAll("\\", "/");
      const name = packages.find(name => path.endsWith(`/node_modules/${name}/dist/${name}.mjs`));
      if (!name) return;
      const embedded = /var (\w+) = ("(?:[^"\\]|\\.)*");\s*styleInject\(\1\);/g;
      const matches = [...code.matchAll(embedded)];
      if (matches.length !== 1 || [...code.matchAll(/\bstyleInject\s*\(/g)].length !== 2) {
        this.error(`Unexpected CSS injection in ${name}; review its distribution before building`);
      }
      const css: unknown = JSON.parse(matches[0][2]);
      if (typeof css !== "string" || !css.trim()) this.error(`Empty embedded CSS in ${name}`);
      const virtual = `${prefix}${name}.css`;
      styles.set(virtual, css);
      return { code: `import ${JSON.stringify(virtual)};\n${code.replace(embedded, "")}`, map: null };
    },
    generateBundle() {
      if (styles.size !== packages.length) this.error("Not all graph library styles were extracted");
    },
  };
}
