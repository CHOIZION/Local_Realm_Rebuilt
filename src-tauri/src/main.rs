#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod archives;
mod calendar;
mod compile;
mod database;
mod documents;
mod entities;
mod export;
mod graph;
mod import;
mod manuscripts;
mod maps;
mod media;
mod ordering;
mod preferences;
mod relationships;
mod sample;
mod schema;
mod search;
mod share_views;
mod tables;
mod terrain;
mod timeline;
mod trash;
mod world;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Manager, State};
use world::{rows, write_atomic, World};

type Reply<T> = std::result::Result<T, String>;
#[derive(Default)]
struct Session(Mutex<Option<World>>);
impl Session {
    fn read<T>(&self, f: impl FnOnce(&World) -> world::Result<T>) -> Reply<T> {
        let guard = self.0.lock().map_err(|_| "World state unavailable")?;
        let world = guard.as_ref().ok_or("Open a realm first")?;
        world.flush_files().map_err(|e| e.to_string())?;
        f(world).map_err(|e| e.to_string())
    }
}
fn prefs(app: &AppHandle) -> world::Result<PathBuf> {
    let path = app.path().app_config_dir()?;
    fs::create_dir_all(&path)?;
    Ok(path)
}
fn read_json(path: &Path, fallback: Value) -> world::Result<Value> {
    match fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(fallback),
        Err(e) => Err(e.into()),
    }
}
fn settings(app: &AppHandle, name: &str, default: Value) -> Reply<Value> {
    let path = prefs(app).map_err(|e| e.to_string())?.join(name);
    read_json(&path, default).map_err(|e| e.to_string())
}
fn save_setting(app: &AppHandle, name: &str, value: &Value) -> Reply<()> {
    let path = prefs(app).map_err(|e| e.to_string())?.join(name);
    write_atomic(
        &path,
        &serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}
#[tauri::command(async)]
fn app_language_get(app: AppHandle) -> Reply<Value> {
    settings(&app, "language.json", Value::Null)
}
#[tauri::command(async)]
fn app_language_set(app: AppHandle, language: String) -> Reply<()> {
    save_setting(&app, "language.json", &json!(language))
}
#[tauri::command(async)]
fn world_recent(app: AppHandle) -> Reply<Value> {
    let mut value = settings(&app, "recent.json", json!([]))?;
    for entry in value.as_array_mut().ok_or("Invalid recent realms file")? {
        entry["missing"] = json!(!Path::new(entry["path"].as_str().unwrap_or(""))
            .join("world.json")
            .is_file());
    }
    Ok(value)
}
#[tauri::command(async)]
fn world_forget(app: AppHandle, path: String) -> Reply<()> {
    let mut recent = world_recent(app.clone())?;
    recent.as_array_mut().unwrap().retain(|r| r["path"] != path);
    save_setting(&app, "recent.json", &recent)
}
mod create;
mod migrations;

#[tauri::command(async)]
fn world_create(app: AppHandle, state: State<Session>, path: String, name: String) -> Reply<Value> {
    let created = create::create(Path::new(&path), &name).map_err(|e| e.to_string())?;
    drop(created);
    world_open(app, state, path)
}
#[tauri::command(async)]
fn world_open(app: AppHandle, state: State<Session>, path: String) -> Reply<Value> {
    let mut guard = state.0.lock().map_err(|_| "World state unavailable")?;
    let world = World::open(Path::new(&path)).map_err(|e| e.to_string())?;
    let info = world.info().map_err(|e| e.to_string())?;
    let mut recent = world_recent(app.clone())?;
    let list = recent.as_array_mut().unwrap();
    list.retain(|r| r["path"] != info["path"]);
    list.insert(0,json!({"id":info["id"],"name":info["name"],"path":info["path"],"opened_at":chrono::Utc::now().to_rfc3339()}));
    save_setting(&app, "recent.json", &recent)?;
    *guard = Some(world);
    Ok(info)
}
#[tauri::command(async)]
fn world_current(state: State<Session>) -> Reply<Value> {
    let guard = state.0.lock().map_err(|_| "World state unavailable")?;
    guard
        .as_ref()
        .map(World::info)
        .transpose()
        .map(|v| v.unwrap_or(Value::Null))
        .map_err(|e| e.to_string())
}
#[tauri::command(async)]
fn world_close(state: State<Session>) -> Reply<()> {
    *state.0.lock().map_err(|_| "World state unavailable")? = None;
    Ok(())
}
#[tauri::command(async)]
fn world_stats(state: State<Session>) -> Reply<Value> {
    state.read(World::stats)
}
#[tauri::command(async)]
fn world_is_empty_dir(path: String) -> Reply<bool> {
    match fs::read_dir(path) {
        Ok(mut d) => Ok(d.next().transpose().map_err(|e| e.to_string())?.is_none()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(e.to_string()),
    }
}
#[tauri::command(async)]
fn ui_state_get(state: State<Session>) -> Reply<Value> {
    state.read(|w| {
        let p = w.root.join(".localrealm-ui.json");
        read_json(
            if p.exists() {
                p
            } else {
                w.root.join(".worldforge-ui.json")
            }
            .as_path(),
            json!({}),
        )
    })
}
#[tauri::command(async)]
fn ui_state_set(state: State<Session>, value: Value) -> Reply<()> {
    state.read(|w| {
        write_atomic(
            &w.root.join(".localrealm-ui.json"),
            &serde_json::to_vec_pretty(&value)?,
        )
    })
}
#[tauri::command(async)]
fn entity_list(state: State<Session>, filter: Value) -> Reply<Vec<Value>> {
    state.read(|w| w.entities(&filter))
}
#[tauri::command(async)]
fn document_get(state: State<Session>, id: String) -> Reply<Value> {
    state.read(|w| w.document(&id))
}
#[tauri::command(async)]
fn document_list(state: State<Session>, kind: Option<String>) -> Reply<Vec<Value>> {
    state.read(|w|rows(&w.db,"SELECT * FROM document WHERE (?1 IS NULL OR kind=?1) ORDER BY sort_order,title COLLATE NOCASE",[kind]))
}
macro_rules! list_command {
    ($name:ident,$sql:literal) => {
        #[tauri::command(async)]
        fn $name(state: State<Session>) -> Reply<Vec<Value>> {
            state.read(|w| rows(&w.db, $sql, []))
        }
    };
}
list_command!(type_list,"SELECT t.*, (SELECT count(*) FROM entity WHERE type_id=t.id) AS entity_count FROM entity_type t ORDER BY sort_order,name COLLATE NOCASE");
list_command!(tag_list,"SELECT t.*, (SELECT count(*) FROM entity_tag WHERE tag_id=t.id) AS count FROM tag t ORDER BY name COLLATE NOCASE");
list_command!(manuscript_list,"SELECT m.*, (SELECT count(*) FROM chapter WHERE manuscript_id=m.id) AS chapter_count,
    (SELECT coalesce(sum(d.word_count),0) FROM chapter c JOIN document d ON d.id=c.document_id WHERE c.manuscript_id=m.id) AS word_count
    FROM manuscript m ORDER BY sort_order,name COLLATE NOCASE");
list_command!(map_list,"SELECT m.*, (SELECT count(*) FROM map_marker WHERE map_id=m.id) AS marker_count FROM map m ORDER BY sort_order,name COLLATE NOCASE");
list_command!(timeline_list,"SELECT t.*, (SELECT count(*) FROM timeline_event WHERE timeline_id=t.id) AS event_count FROM timeline t ORDER BY sort_order,name COLLATE NOCASE");
list_command!(
    calendar_list,
    "SELECT * FROM calendar ORDER BY is_default DESC,name COLLATE NOCASE"
);
list_command!(
    table_list,
    "SELECT * FROM table_view ORDER BY sort_order,name COLLATE NOCASE"
);
list_command!(
    tree_list,
    "SELECT * FROM relationship_tree ORDER BY sort_order,name COLLATE NOCASE"
);

#[tauri::command(async)]
fn type_get(state: State<Session>, id: String) -> Reply<Value> {
    state.read(|w| {
        rows(&w.db, "SELECT * FROM entity_type WHERE id=?1", [id])?
            .pop()
            .ok_or_else(|| "Category not found".into())
    })
}
#[tauri::command(async)]
fn map_get(state: State<Session>, id: String) -> Reply<Value> {
    state.read(|w| maps::get(w, &id))
}
mod grid;

#[tauri::command(async)]
fn map_grid_get(state: State<Session>, id: String) -> Reply<tauri::ipc::Response> {
    state.read(|w| Ok(tauri::ipc::Response::new(grid::read(w, &id)?.cells)))
}

#[tauri::command(async)]
fn map_grid_save(state: State<Session>, request: tauri::ipc::Request<'_>) -> Reply<()> {
    let id = request
        .headers()
        .get("map-id")
        .and_then(|h| h.to_str().ok())
        .ok_or("Missing map id")?;
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("Expected binary map data".into());
    };
    state.read(|w| grid::save(w, id, bytes))
}

fn main() {
    // A portable release can carry a fixed WebView2 runtime beside the executable.
    // Otherwise WebView2 selects the installed Evergreen runtime.
    if std::env::var_os("WEBVIEW2_BROWSER_EXECUTABLE_FOLDER").is_none() {
        if let Some(directory) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_owned))
        {
            if let Ok(entries) = fs::read_dir(directory) {
                for entry in entries.flatten() {
                    if entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("Microsoft.WebView2.FixedVersionRuntime.")
                        && entry.path().join("msedgewebview2.exe").is_file()
                    {
                        std::env::set_var("WEBVIEW2_BROWSER_EXECUTABLE_FOLDER", entry.path());
                        break;
                    }
                }
            }
        }
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Session::default())
        .invoke_handler(tauri::generate_handler![
            app_language_get,
            app_language_set,
            world_recent,
            world_forget,
            world_open,
            world_create,
            world_close,
            world_current,
            world_stats,
            world_is_empty_dir,
            ui_state_get,
            ui_state_set,
            entity_list,
            type_list,
            type_get,
            tag_list,
            relationships::relation_kinds,
            manuscript_list,
            map_list,
            map_get,
            map_grid_get,
            map_grid_save,
            timeline_list,
            calendar_list,
            table_list,
            tree_list,
            document_get,
            entities::entity_get,
            entities::entity_create,
            entities::entity_update,
            entities::entity_set_properties,
            entities::entity_backlinks,
            entities::entity_lookup,
            entities::entity_duplicate,
            schema::type_properties,
            schema::type_create,
            schema::type_update,
            schema::type_set_layout,
            schema::type_delete,
            schema::property_create,
            schema::property_update,
            schema::property_delete,
            schema::property_data_types,
            documents::document_save,
            documents::note_create,
            documents::note_rename,
            documents::revision_list,
            documents::revision_body,
            documents::revision_restore,
            documents::markdown_preview,
            search::search_query,
            search::search_rebuild,
            relationships::relation_kind_create,
            relationships::relation_kind_delete,
            relationships::relationship_create,
            relationships::relationship_delete,
            trash::entity_delete,
            trash::document_delete,
            trash::trash_list,
            trash::trash_restore,
            trash::trash_purge,
            ordering::types_reorder,
            ordering::entities_reorder,
            ordering::notes_reorder,
            ordering::map_layers_reorder,
            ordering::map_regions_reorder,
            ordering::property_reorder,
            ordering::part_reorder,
            preferences::world_rename,
            preferences::world_describe,
            preferences::dictionary_get,
            preferences::dictionary_set,
            preferences::world_background_get,
            preferences::world_background_set,
            preferences::graph_arrangement,
            preferences::graph_arrangement_save,
            preferences::reveal_in_file_manager,
            manuscripts::manuscript_get,
            manuscripts::manuscript_create,
            manuscripts::manuscript_update,
            manuscripts::manuscript_set_layout,
            manuscripts::manuscript_set_ebook,
            manuscripts::manuscript_delete,
            manuscripts::chapter_create,
            manuscripts::chapter_get,
            manuscripts::chapter_update,
            manuscripts::chapter_move,
            manuscripts::chapter_delete,
            manuscripts::part_create,
            manuscripts::part_rename,
            manuscripts::part_delete,
            manuscripts::manuscript_statuses,
            manuscripts::manuscript_reports,
            media::media_import,
            media::media_list,
            media::media_usage,
            media::media_unused,
            media::media_attach,
            media::media_detach,
            media::media_caption,
            media::media_delete,
            media::media_path,
            media::media_read_brush,
            media::brush_library_list,
            media::brush_library_read,
            maps::map_create,
            maps::map_update,
            maps::map_set_image,
            maps::map_delete,
            maps::map_layer_create,
            maps::map_layer_update,
            maps::map_layer_delete,
            maps::map_marker_move,
            maps::map_stamp_image_put,
            maps::map_stamp_image_get,
            maps::map_feature_dates,
            maps::map_outline_add,
            maps::map_outline_update,
            maps::map_outline_delete,
            maps::map_placements,
            maps::map_marker_create,
            maps::map_marker_update,
            maps::map_marker_delete,
            maps::map_region_create,
            maps::map_region_update,
            maps::map_region_delete,
            maps::map_stamp_create,
            maps::map_stamp_update,
            maps::map_stamp_delete,
            calendar::calendar_default,
            calendar::calendar_create,
            calendar::calendar_update,
            calendar::calendar_delete,
            calendar::calendar_eras,
            calendar::calendar_format,
            calendar::timeline_ticks,
            timeline::timeline_get,
            timeline::timeline_create,
            timeline::timeline_update,
            timeline::timeline_delete,
            timeline::timeline_event_create,
            timeline::timeline_event_update,
            timeline::timeline_event_delete,
            timeline::timeline_appearances,
            timeline::map_pins,
            timeline::map_events,
            terrain::map_create_painted,
            terrain::map_generate,
            terrain::map_erode,
            terrain::map_relief,
            terrain::map_shape_set,
            terrain::map_biomes_set,
            terrain::map_export_image,
            graph::graph_build,
            graph::graph_degree,
            graph::tree_get,
            graph::tree_build,
            graph::tree_create,
            graph::tree_update,
            graph::tree_delete,
            graph::tree_kinds,
            graph::tree_memberships,
            tables::table_columns,
            tables::table_preview,
            tables::table_run,
            tables::table_get,
            tables::table_create,
            tables::table_update,
            tables::table_delete,
            archives::backup_create,
            archives::backup_list,
            archives::backup_delete,
            archives::backup_restore,
            export::export_world,
            export::import_world,
            import::import_scan,
            import::import_run,
            compile::manuscript_pages,
            compile::manuscript_compile,
            compile::manuscript_formats,
            compile::manuscript_fonts,
            sample::world_create_sample,
            document_list
        ])
        .run(tauri::generate_context!())
        .expect("Local Realm startup failed");
}
#[cfg(test)]
mod tests;
