import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import test from "node:test";

const scriptPath = join(dirname(fileURLToPath(import.meta.url)), "set-updater-notes.mjs");

function fixture(manifest, notes) {
  const directory = mkdtempSync(join(tmpdir(), "displaymux-updater-notes-"));
  const manifestPath = join(directory, "latest.json");
  const notesPath = join(directory, "release-notes.md");
  writeFileSync(manifestPath, JSON.stringify(manifest));
  writeFileSync(notesPath, notes);
  return { manifestPath, notesPath };
}

function run(manifestPath, notesPath) {
  return spawnSync(
    process.execPath,
    [scriptPath, "--manifest", manifestPath, "--notes", notesPath],
    { encoding: "utf8" },
  );
}

test("copies release notes without changing updater platform data", () => {
  const original = {
    version: "1.2.3",
    notes: "",
    pub_date: "2026-09-13T00:00:00.000Z",
    platforms: {
      "windows-x86_64-nsis": { url: "https://example.test/app.exe", signature: "signature" },
    },
  };
  const { manifestPath, notesPath } = fixture(original, "# Release\n\nFixed update notes.\n");

  const result = run(manifestPath, notesPath);

  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(JSON.parse(readFileSync(manifestPath, "utf8")), {
    ...original,
    notes: "# Release\n\nFixed update notes.",
  });
});

test("rejects a manifest without platform packages", () => {
  const { manifestPath, notesPath } = fixture({ version: "1.2.3", platforms: {} }, "Release notes");

  const result = run(manifestPath, notesPath);

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /does not contain any platform packages/);
});

test("rejects empty release notes", () => {
  const { manifestPath, notesPath } = fixture(
    { version: "1.2.3", platforms: { "darwin-aarch64": {} } },
    "  \n",
  );

  const result = run(manifestPath, notesPath);

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /Release notes are empty/);
});
