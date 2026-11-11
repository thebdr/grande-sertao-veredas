// Supervisão's page: built into dist/ with fixed names (the crate compiles them in with
// include_str!), with two hashes beside them, both recomputed by the crate's component test
// (src/web.rs states the algorithm: FNV-1a 64 over each file's relative path, a NUL, its bytes
// with every CR LF pair read as LF, a NUL, in path order):
//   dist/source.hash  the sources the page was built from (SOURCE_FILES and everything under src/)
//   dist/output.hash  the page as written (OUTPUT_FILES)
// The build reads nothing else: no .env, no public/ folder, no PostCSS file, no module and no
// file a stylesheet pulls in outside src/ and the hashed files (refused; node_modules/ too, whose
// packages the hash never sees), no asset inlined (an asset is a file of its own, so the
// three-file rule refuses it), and it emits the three files alone (refused otherwise). The hashes
// are written once the bundle is on disk: a failed build never writes them.
import { defineConfig, type Plugin } from "vite";
import { readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const SOURCE_FILES = ["index.html", "package.json", "package-lock.json", "tsconfig.json", "vite.config.ts"];
const OUTPUT_FILES = ["assets/app.css", "assets/app.js", "index.html"];

function walk(dir: string, rel: string, out: string[]) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    const r = `${rel}/${name}`;
    if (statSync(p).isDirectory()) walk(p, r, out);
    else out.push(r);
  }
}

function fnv(bytes: Uint8Array, h: bigint): bigint {
  for (const b of bytes) {
    h ^= BigInt(b);
    h = (h * 0x100000001b3n) & 0xffffffffffffffffn;
  }
  return h;
}

// The hash of the files under root named by rel, sorted as Rust sorts strings: by bytes.
function hashFiles(root: string, rel: string[]): string {
  const files = [...rel].sort((a, b) => Buffer.compare(Buffer.from(a), Buffer.from(b)));
  const enc = new TextEncoder();
  let h = 0xcbf29ce484222325n;
  for (const r of files) {
    const raw = readFileSync(join(root, r));
    const lf = raw.filter((b, i) => !(b === 13 && raw[i + 1] === 10));
    h = fnv(enc.encode(r), h);
    h = fnv(new Uint8Array([0]), h);
    h = fnv(lf, h);
    h = fnv(new Uint8Array([0]), h);
  }
  return h.toString(16).padStart(16, "0");
}

export function sourceHash(root: string): string {
  const files = [...SOURCE_FILES];
  walk(join(root, "src"), "src", files);
  return hashFiles(root, files);
}

const norm = (p: string) => resolve(p).replace(/\\/g, "/").toLowerCase();

function stamp(): Plugin {
  const src = norm(join(__dirname, "src")) + "/";
  const hashed = SOURCE_FILES.map((f) => norm(join(__dirname, f)));
  return {
    name: "source-hash",
    // After vite's own plugins, so the bundle it judges holds index.html.
    enforce: "post",
    generateBundle(_options, bundle) {
      // Every module read, and every file one pulls in (a stylesheet's @import, what vite
      // watches), is a hashed source.
      for (const id of [...this.getModuleIds(), ...this.getWatchFiles()]) {
        if (id.startsWith("\0")) continue;
        const p = norm(id.split("?")[0]);
        if (!(p.startsWith(src) || hashed.includes(p))) {
          this.error(`the page reads ${id}, outside src/ and the hashed files: move it under src/`);
        }
      }
      // The page is the three files the host serves, and nothing else.
      const names = Object.keys(bundle).sort();
      if (JSON.stringify(names) !== JSON.stringify(OUTPUT_FILES)) {
        this.error(`the page must build into ${OUTPUT_FILES.join(", ")} alone, not ${names.join(", ")}`);
      }
    },
    writeBundle() {
      const dist = join(__dirname, "dist");
      writeFileSync(join(dist, "source.hash"), sourceHash(__dirname));
      writeFileSync(join(dist, "output.hash"), hashFiles(dist, OUTPUT_FILES));
    },
  };
}

export default defineConfig({
  base: "/",
  plugins: [stamp()],
  envDir: false,
  publicDir: false,
  css: { postcss: {} },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
    modulePreload: false,
    assetsInlineLimit: 0,
    rollupOptions: {
      output: {
        entryFileNames: "assets/app.js",
        chunkFileNames: "assets/app-[name].js",
        assetFileNames: (info) => (info.names?.some((n) => n.endsWith(".css")) ? "assets/app.css" : "assets/[name][extname]"),
      },
    },
  },
});
