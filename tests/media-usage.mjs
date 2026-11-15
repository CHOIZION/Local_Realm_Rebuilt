import assert from "node:assert/strict";
import { test } from "node:test";
import { openMediaUsage } from "../frontend/media-usage.mjs";

test("media references open their owning view", () => {
  const calls = [];
  const store = {
    openEntity: (...args) => calls.push(["entity", ...args]),
    openManuscript: (...args) => calls.push(["manuscript", ...args]),
    open: (...args) => calls.push(args),
  };
  openMediaUsage(store, { entity_id: "character" });
  openMediaUsage(store, { kind: "document", manuscript_id: "book" });
  openMediaUsage(store, { kind: "document", document_id: "note", title: "Notes" });
  openMediaUsage(store, { kind: "map", id: "map", title: "Coast" });
  assert.deepEqual(calls, [
    ["entity", "character"], ["manuscript", "book"],
    ["note", "note", "Notes", "Note"], ["map", "map", "Coast", "Map"],
  ]);
});
