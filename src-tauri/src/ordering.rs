use crate::database;
use tauri::State;
macro_rules! order_command {
    ($name:ident,$table:literal) => {
        #[tauri::command(async)]
        pub fn $name(state: State<crate::Session>, ids: Vec<String>) -> crate::Reply<()> {
            state.read(|w| w.transaction(|db| database::reorder(db, $table, &ids)))
        }
    };
}
order_command!(types_reorder, "entity_type");
order_command!(entities_reorder, "entity");
order_command!(notes_reorder, "document");
order_command!(map_layers_reorder, "map_layer");
order_command!(map_regions_reorder, "map_region");
order_command!(property_reorder, "property_definition");
order_command!(part_reorder, "manuscript_part");
