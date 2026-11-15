// The store replaces the complete entity list after a mutation and when closing a world.
// Grouping preserves the backend order, including drag-and-drop ordering.
export function indexEntities(entities) {
  const byId = new Map();
  const byType = new Map();
  for (const entity of entities) {
    if (!byId.has(entity.id)) byId.set(entity.id, entity);
    let group = byType.get(entity.type_id);
    if (!group) {
      group = [];
      byType.set(entity.type_id, group);
    }
    group.push(entity);
  }
  return { byId, byType };
}
