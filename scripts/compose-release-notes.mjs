import { readFileSync, writeFileSync } from "node:fs";

function argument(name) {
  const index = process.argv.indexOf(name);
  if (index < 0 || !process.argv[index + 1]) throw new Error(`Missing ${name}`);
  return process.argv[index + 1];
}

const tag = argument("--tag");
if (!/^v\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(tag)) throw new Error(`Invalid release tag: ${tag}`);
const repository = argument("--repository");
if (!/^[0-9A-Za-z_.-]+\/[0-9A-Za-z_.-]+$/.test(repository)) throw new Error(`Invalid repository: ${repository}`);
const assets = JSON.parse(readFileSync(argument("--assets"), "utf8"));
const englishNotes = readFileSync(`.github/release-notes/${tag}.en.md`, "utf8").trim();
const traditionalChineseNotes = readFileSync(`.github/release-notes/${tag}.zh-TW.md`, "utf8").trim();

if (!Array.isArray(assets)) throw new Error("Release assets must be a JSON array");

const findAsset = (predicate) => assets.find((asset) => typeof asset?.name === "string" && predicate(asset.name));
const windowsInstaller = findAsset((name) => name.endsWith(".exe"));
const macUniversalDmg = findAsset((name) => name.endsWith(".dmg") && name.toLowerCase().includes("universal"));

if (!windowsInstaller) throw new Error(`Windows installer is missing from ${tag}`);
if (!macUniversalDmg) throw new Error(`macOS Universal DMG is missing from ${tag}`);

const downloadUrl = (asset) =>
  `https://github.com/${repository}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(asset.name)}`;

const sections = [
  "## Direct Downloads / 直接下載",
  "",
  `- 💻 [Download for Windows (64-bit) / 下載 Windows 64 位元版](${downloadUrl(windowsInstaller)})`,
  `- 🍎 [Download for macOS (Universal) / 下載 macOS 通用版](${downloadUrl(macUniversalDmg)})`,
  "",
  "## English",
  "",
  englishNotes,
  "",
  "---",
  "",
  "## 繁體中文",
  "",
  traditionalChineseNotes,
];

writeFileSync(argument("--output"), `${sections.join("\n")}\n`);
