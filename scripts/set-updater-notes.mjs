import { readFileSync, writeFileSync } from "node:fs";

function argument(name) {
  const index = process.argv.indexOf(name);
  if (index < 0 || !process.argv[index + 1]) throw new Error(`Missing ${name}`);
  return process.argv[index + 1];
}

const manifestPath = argument("--manifest");
const notesPath = argument("--notes");
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
const notes = readFileSync(notesPath, "utf8").trim();

if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)) {
  throw new Error("Updater manifest must be a JSON object");
}
if (typeof manifest.version !== "string" || !manifest.version.trim()) {
  throw new Error("Updater manifest version is missing");
}
if (!manifest.platforms || typeof manifest.platforms !== "object" || Array.isArray(manifest.platforms)) {
  throw new Error("Updater manifest platforms are missing");
}
if (Object.keys(manifest.platforms).length === 0) {
  throw new Error("Updater manifest does not contain any platform packages");
}
if (!notes) {
  throw new Error("Release notes are empty");
}

manifest.notes = notes;
writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
