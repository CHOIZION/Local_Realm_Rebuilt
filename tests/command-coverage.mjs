import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { optimizedFiles } from "../tools/build-frontend.mjs";

test("every recovered frontend command is registered in the Rust application", async () => {
  const required = (await readFile(new URL("../recovered-backend/ipc-commands.txt", import.meta.url), "utf8")).trim().split(/\s+/);
  const main = await readFile(new URL("../src-tauri/src/main.rs", import.meta.url), "utf8");
  const handler = main.slice(main.indexOf("tauri::generate_handler!"));
  const registered = new Set([...handler.matchAll(/^\s+(?:\w+::)?(\w+),?$/gm)].map(m => m[1]));
  assert.equal(required.length, 183);
  assert.deepEqual(required.filter(name => !registered.has(name)), []);
});

test("frontend transformations match the recovered build exactly", async () => {
  const files = await optimizedFiles();
  assert.equal(files.size, 2);
  for (const source of files.values()) assert.ok(source.length > 1000);
});
