// Pins the ZONOS3-Mini weights Booth downloads to one Hugging Face commit.
//
//   node scripts/models-lock.mjs [revision]   write crates/booth-core/models.lock.json (default: today's main)
//   node scripts/models-lock.mjs --check      compare the lock with Hugging Face's main; exit 1 and print
//                                            what changed if a file Booth uses differs (the daily watch)
//
// The lock lists every file of every pack with its size and SHA-256, plus the names of its copies on
// the GitHub mirror release. Files over GitHub's 2 GiB asset limit are mirrored in parts.
// After changing the lock, the "Mirror the model files" workflow uploads the new revision.
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";

const REPO = "Zyphra/ZONOS3-Mini";
const MIRROR_REPO = "stepupgaming/booth";
const PART_SIZE = 2_000_000_000; // under GitHub's 2 GiB per-asset limit
const LOCK = new URL("../crates/booth-core/models.lock.json", import.meta.url);

// Paths under gguf/. A trailing slash takes the whole folder.
const PACKS = {
  core: ["backbone-ptq.gguf", "zac-keep2.gguf", "spk-encoder.gguf", "steer.gguf", "voices/"],
  exact: ["zac-encoder.gguf", "ssl-teacher.gguf"],
  full_decoder: ["zac.gguf"],
};

async function get(url, as = "json") {
  for (let attempt = 1; ; attempt++) {
    try {
      const r = await fetch(url);
      if (!r.ok) throw new Error(`${r.status} ${r.statusText}`);
      return as === "json" ? await r.json() : Buffer.from(await r.arrayBuffer());
    } catch (e) {
      if (attempt >= 4) throw new Error(`${url}: ${e.message}`);
      await new Promise((ok) => setTimeout(ok, 2000 * attempt));
    }
  }
}

const enc = (rel) => rel.split("/").map(encodeURIComponent).join("/");
const assetName = (rel) => rel.replace(/\//g, "--").replace(/[^A-Za-z0-9._-]/g, "_");

async function build(revision) {
  const sha = (await get(`https://huggingface.co/api/models/${REPO}/revision/${revision}`)).sha;
  const tree = await get(`https://huggingface.co/api/models/${REPO}/tree/${sha}/gguf?recursive=1`);
  const files = tree.filter((e) => e.type === "file");
  const packs = {};
  for (const [pack, wants] of Object.entries(PACKS)) {
    packs[pack] = [];
    for (const want of wants) {
      const hits = files.filter((e) =>
        want.endsWith("/") ? e.path.startsWith(`gguf/${want}`) : e.path === `gguf/${want}`,
      );
      if (!hits.length) throw new Error(`gguf/${want} is not in ${REPO}@${sha}`);
      for (const e of hits.sort((a, b) => a.path.localeCompare(b.path))) {
        const rel = e.path.slice("gguf/".length);
        let sha256 = e.lfs?.oid;
        if (!sha256) {
          // Small files live in git, not LFS, so the Hub gives no SHA-256: hash them here.
          const body = await get(`https://huggingface.co/${REPO}/resolve/${sha}/gguf/${enc(rel)}`, "bytes");
          if (body.length !== e.size) throw new Error(`${rel}: got ${body.length} of ${e.size} bytes`);
          sha256 = createHash("sha256").update(body).digest("hex");
        }
        const name = assetName(rel);
        const parts = [];
        if (e.size <= PART_SIZE) parts.push({ name, size: e.size });
        else
          for (let i = 0, off = 0; off < e.size; i++, off += PART_SIZE)
            parts.push({ name: `${name}.part${i + 1}`, size: Math.min(PART_SIZE, e.size - off) });
        packs[pack].push({ rel, size: e.size, sha256, mirror: parts });
      }
    }
  }
  return {
    repo: REPO,
    revision: sha,
    mirror: `https://github.com/${MIRROR_REPO}/releases/download/models-${sha.slice(0, 10)}`,
    packs,
  };
}

const arg = process.argv[2];
if (arg === "--check") {
  const lock = JSON.parse(readFileSync(LOCK, "utf8"));
  const main = await build("main");
  const key = (f) => `${f.size} ${f.sha256}`;
  const changes = [];
  for (const pack of Object.keys(PACKS)) {
    const was = new Map(lock.packs[pack].map((f) => [f.rel, f]));
    const now = new Map(main.packs[pack].map((f) => [f.rel, f]));
    for (const [rel, f] of now)
      if (!was.has(rel)) changes.push(`+ ${pack}: ${rel} (new, ${f.size} bytes)`);
      else if (key(was.get(rel)) !== key(f)) changes.push(`~ ${pack}: ${rel} (changed, ${was.get(rel).size} → ${f.size} bytes)`);
    for (const rel of was.keys()) if (!now.has(rel)) changes.push(`- ${pack}: ${rel} (removed or renamed)`);
  }
  console.log(`Pinned: ${lock.revision}\nHugging Face main: ${main.revision}`);
  if (!changes.length) {
    console.log("No file Booth uses has changed.");
  } else {
    console.log(`${changes.length} change(s) to files Booth uses:\n${changes.join("\n")}`);
    process.exit(1);
  }
} else {
  const lock = await build(arg || "main");
  writeFileSync(LOCK, JSON.stringify(lock, null, 2) + "\n");
  const n = Object.values(lock.packs).flat();
  console.log(`Pinned ${REPO}@${lock.revision}: ${n.length} files, ${(n.reduce((s, f) => s + f.size, 0) / 1e9).toFixed(2)} GB`);
}
