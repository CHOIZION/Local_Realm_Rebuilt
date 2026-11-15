export function openMediaUsage(store, usage) {
  if (usage.entity_id) return store.openEntity(usage.entity_id);
  if (usage.manuscript_id) return store.openManuscript(usage.manuscript_id);
  if (usage.kind === "document") {
    return store.open("note", usage.document_id, usage.title, "Note");
  }
  if (usage.kind === "manuscript") return store.openManuscript(usage.id);
  if (usage.kind === "map") return store.open("map", usage.id, usage.title, "Map");
}
