import { copyFileSync, existsSync, mkdirSync, readdirSync } from "node:fs";
import { join } from "node:path";

const root = join("src-tauri", "target", "release", "build");
const dest = join("src-tauri", "native");
const native = (name) => /\.(dll|dylib)$/i.test(name) || /\.so(\.\d+)*$/.test(name);

mkdirSync(dest, { recursive: true });

const outs = existsSync(root)
  ? readdirSync(root)
      .filter((d) => d.startsWith("foundry-local-sdk-"))
      .map((d) => join(root, d, "out"))
      .filter((d) => existsSync(d))
  : [];

let copied = 0;
for (const dir of outs) {
  for (const file of readdirSync(dir).filter(native)) {
    copyFileSync(join(dir, file), join(dest, file));
    copied += 1;
  }
}

if (!copied) {
  console.error("No Foundry Local native libraries found. Run a release build with internet access first.");
  process.exit(1);
}
console.log(`Gathered ${copied} native libraries into ${dest}`);
