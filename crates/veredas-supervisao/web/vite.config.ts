// Supervisão's page: built into dist/ with fixed names (the crate compiles them in with
// include_str!), and dist/source.hash recording the sources it was built from, the hash the
// crate's component test recomputes (src/web.rs states the algorithm: FNV-1a 64 over each
// source's relative path, a NUL, its text with CRLF read as LF, a NUL, in path order).
import { defineConfig, type Plugin } from "vite";
import { readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const SOURCE_FILES = ["index.html", "package.json", "package-lock.json", "tsconfig.json", "vite.config.ts"];

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

export function sourceHash(root: string): string {
  const files = [...SOURCE_FILES];
  walk(join(root, "src"), "src", files);
  // Sorted as Rust sorts strings: by bytes.
  files.sort((a, b) => Buffer.compare(Buffer.from(a), Buffer.from(b)));
  const enc = new TextEncoder();
  let h = 0xcbf29ce484222325n;
  for (const rel of files) {
    const text = readFileSync(join(root, rel), "utf8").replace(/\r\n/g, "\n");
    h = fnv(enc.encode(rel), h);
    h = fnv(new Uint8Array([0]), h);
    h = fnv(enc.encode(text), h);
    h = fnv(new Uint8Array([0]), h);
  }
  return h.toString(16).padStart(16, "0");
}

function stamp(): Plugin {
  return {
    name: "source-hash",
    closeBundle() {
      writeFileSync(join(__dirname, "dist", "source.hash"), sourceHash(__dirname));
    },
  };
}

export default defineConfig({
  base: "/",
  plugins: [stamp()],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
    modulePreload: false,
    rollupOptions: {
      output: {
        entryFileNames: "assets/app.js",
        chunkFileNames: "assets/app-[name].js",
        assetFileNames: (info) => (info.names?.some((n) => n.endsWith(".css")) ? "assets/app.css" : "assets/[name][extname]"),
      },
    },
  },
});
