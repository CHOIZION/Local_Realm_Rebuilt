use super::*;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

#[test]
fn bundled_sample_opens_without_original_installation() -> world::Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("Sample");
    sample::create(&path)?;
    let world = World::open(&path)?;
    assert_eq!(world.entities(&json!({}))?.len(), 11);
    assert_eq!(
        world
            .db
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))?,
        "ok"
    );
    assert!(rows(&world.db, "PRAGMA foreign_key_check", [])?.is_empty());
    for map in rows(&world.db, "SELECT id FROM map WHERE kind='painted'", [])? {
        let grid = grid::read(&world, map["id"].as_str().unwrap())?;
        assert_eq!(grid.cells.len(), (grid.width * grid.height) as usize);
    }
    let wiki_path = temp.path().join("sample.html");
    export::export(&world, &json!({"format":"html","destination":wiki_path}))?;
    let wiki = fs::read_to_string(wiki_path)?;
    assert!(wiki.contains("<svg"));
    assert!(wiki.contains("<table"));
    Ok(())
}

#[test]
fn frontend_ipc_arguments_and_binary_maps_round_trip() -> world::Result<()> {
    use tauri::{
        ipc::{CallbackFn, InvokeBody, InvokeResponseBody},
        test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY},
    };
    let temp = tempfile::tempdir()?;
    let world = create::create(&temp.path().join("IPC"), "IPC")?;
    let app = mock_builder()
        .manage(Session(Mutex::new(Some(world))))
        .invoke_handler(tauri::generate_handler![
            type_list,
            entities::entity_create,
            documents::note_create,
            documents::document_save,
            document_get,
            terrain::map_create_painted,
            map_grid_get,
            map_grid_save
        ])
        .build(mock_context(noop_assets()))?;
    // MockRuntime does not create an operating-system window or start WebView2.
    let view = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build()?;
    let invoke = |cmd: &str, body: InvokeBody, map_id: Option<&str>| {
        let mut headers = tauri::http::HeaderMap::new();
        if let Some(id) = map_id {
            headers.insert("map-id", id.parse().unwrap());
        }
        get_ipc_response(
            &view,
            tauri::webview::InvokeRequest {
                cmd: cmd.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: "http://tauri.localhost".parse().unwrap(),
                body,
                headers,
                invoke_key: INVOKE_KEY.into(),
            },
        )
    };
    let json_call = |cmd: &str, body: Value| -> Value {
        invoke(cmd, InvokeBody::Json(body), None)
            .unwrap_or_else(|e| panic!("{cmd}: {e}"))
            .deserialize()
            .unwrap()
    };
    let types = json_call("type_list", json!({}));
    let entity = json_call(
        "entity_create",
        json!({"typeId":types[0]["id"],"name":"새 인물"}),
    );
    assert_eq!(entity["name"], "새 인물");
    let note = json_call("note_create", json!({"title":"항해 기록"}));
    json_call(
        "document_save",
        json!({"id":note["id"],"body":"별을 따라 항해했다."}),
    );
    assert_eq!(
        json_call("document_get", json!({"id":note["id"]}))["body"],
        "별을 따라 항해했다."
    );
    let map = json_call(
        "map_create_painted",
        json!({"name":"IPC map","options":{"cols":32,"rows":16,"fill":"ocean"}}),
    );
    let id = map["id"].as_str().unwrap();
    let cells = vec![1; 32 * 16];
    invoke("map_grid_save", InvokeBody::Raw(cells.clone()), Some(id)).unwrap();
    match invoke("map_grid_get", InvokeBody::Json(json!({"id":id})), None).unwrap() {
        InvokeResponseBody::Raw(bytes) => assert_eq!(bytes, cells),
        _ => panic!("Grid must use a binary IPC response"),
    }
    assert!(invoke("map_grid_save", InvokeBody::Raw(vec![1; 3]), Some(id)).is_err());
    Ok(())
}

fn root_with_schema(path: &Path) -> world::Result<()> {
    fs::create_dir_all(path.join("maps"))?;
    fs::write(
        path.join("world.json"),
        r#"{"name":"Grid fixture","id":"fixture","schema_version":20}"#,
    )?;
    let db = Connection::open(path.join("world.db"))?;
    let mut migrations = fs::read_dir(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../recovered-backend/migrations"),
    )?
    .map(|p| p.map(|e| e.path()))
    .collect::<std::io::Result<Vec<_>>>()?;
    migrations.sort();
    assert_eq!(migrations.len(), 20);
    for migration in migrations {
        db.execute_batch(&fs::read_to_string(migration)?)?;
    }
    db.pragma_update(None, "user_version", 20)?;
    db.execute("INSERT INTO map (id,name,width,height,kind,created_at,updated_at) VALUES ('fixture','Grid',4,2,'painted','','')",[])?;
    let file = fs::File::create(path.join("maps/fixture.png"))?;
    let mut encoder = png::Encoder::new(file, 4, 2);
    encoder.set_color(png::ColorType::Indexed);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_palette(vec![0, 0, 0, 40, 130, 200]);
    encoder.set_trns(vec![0, 255]);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&[0; 8])?;
    writer.finish()?;
    Ok(())
}

#[test]
fn grid_preserves_palette_and_rejects_incomplete_saves() -> world::Result<()> {
    let root = tempfile::tempdir()?;
    root_with_schema(root.path())?;
    let w = World::open(root.path())?;
    assert_eq!(w.info()?["stats"]["entities"], 0);
    let before = fs::read(root.path().join("maps/fixture.png"))?;
    assert!(grid::save(&w, "fixture", &[1; 7]).is_err());
    assert!(grid::save(&w, "fixture", &[2; 8]).is_err());
    grid::save(&w, "fixture", &[0; 8])?;
    assert_eq!(before, fs::read(root.path().join("maps/fixture.png"))?);
    let stroke = [0, 1, 1, 0, 0, 0, 0, 0];
    grid::save(&w, "fixture", &stroke)?;
    let w = World::open(root.path())?;
    assert_eq!(grid::read(&w, "fixture")?.cells, stroke);
    let reader = png::Decoder::new(std::io::BufReader::new(fs::File::open(
        root.path().join("maps/fixture.png"),
    )?))
    .read_info()?;
    assert_eq!(
        reader.info().palette.as_deref(),
        Some([0, 0, 0, 40, 130, 200].as_slice())
    );
    assert_eq!(reader.info().trns.as_deref(), Some([0, 255].as_slice()));
    Ok(())
}

#[test]
#[ignore = "Set LOCAL_REALM_REFERENCE to a sample created by 1.9.0 and run with --ignored"]
fn original_sample_round_trip() -> world::Result<()> {
    let sample = PathBuf::from(std::env::var("LOCAL_REALM_REFERENCE")?);
    let w = World::open(&sample)?;
    assert_eq!(w.info()?["name"], "The Ashen Reach (sample)");
    let stats = w.stats()?;
    assert_eq!(stats["entities"], 11);
    assert_eq!(stats["types"], 7);
    assert_eq!(stats["relationships"], 9);
    assert_eq!(stats["words"], 412);
    assert_eq!(w.entities(&json!({"sort":"order","limit":5000}))?.len(), 11);
    let doc: String = w.db.query_row(
        "SELECT id FROM document WHERE kind='note' AND title='Start here'",
        [],
        |r| r.get(0),
    )?;
    assert!(!w.document(&doc)?["body"].as_str().unwrap().is_empty());
    let id: String =
        w.db.query_row("SELECT id FROM map WHERE kind='painted' LIMIT 1", [], |r| {
            r.get(0)
        })?;
    let original = grid::read(&w, &id)?.cells;
    let dest = tempfile::tempdir()?;
    fs::create_dir(dest.path().join("maps"))?;
    // VACUUM INTO includes committed WAL pages even while the reference is open.
    w.db.execute(
        "VACUUM INTO ?1",
        [dest.path().join("world.db").to_string_lossy().as_ref()],
    )?;
    fs::copy(sample.join("world.json"), dest.path().join("world.json"))?;
    let filename = format!("maps/{id}.png");
    fs::copy(sample.join(&filename), dest.path().join(&filename))?;
    let copy = World::open(dest.path())?;
    let mut changed = original.clone();
    changed[0] = if changed[0] == 0 { 1 } else { 0 };
    grid::save(&copy, &id, &changed)?;
    assert_eq!(grid::read(&copy, &id)?.cells, changed);
    grid::save(&copy, &id, &original)?;
    assert_eq!(grid::read(&copy, &id)?.cells, original);
    Ok(())
}
#[test]
fn create_matches_default_realm_and_leaves_existing_files_intact() -> world::Result<()> {
    let dir = tempfile::tempdir()?;
    let dest = dir.path().join("새 세계");
    let w = create::create(&dest, "새 세계")?;
    assert_eq!(w.info()?["stats"]["types"], 7);
    assert_eq!(
        w.db.query_row("SELECT count(*) FROM property_definition", [], |r| r
            .get::<_, i64>(0))?,
        24
    );
    assert_eq!(
        w.db.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r
            .get::<_, i64>(
            0
        ))?,
        0
    );
    assert_eq!(w.db.query_row("SELECT count(*) FROM property_definition p WHERE json_type(p.config,'$.target_type_id') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM entity_type t WHERE t.id=json_extract(p.config,'$.target_type_id'))",[],|r|r.get::<_,i64>(0))?,0);
    let manifest = fs::read(dest.join("world.json"))?;
    assert!(create::create(&dest, "replacement").is_err());
    assert_eq!(fs::read(dest.join("world.json"))?, manifest);
    let empty = dir.path().join("Already created folder");
    fs::create_dir(&empty)?;
    assert_eq!(
        create::create(&empty, "Empty folder")?.info()?["name"],
        "Empty folder"
    );
    Ok(())
}
#[test]
fn editing_search_revision_and_trash_survive_reopening() -> world::Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("Writing");
    let w = create::create(&path, "Writing")?;
    let type_id: String = w.db.query_row(
        "SELECT id FROM entity_type WHERE name='Character'",
        [],
        |r| r.get(0),
    )?;
    let first = w.transaction(|db| entities::create_in(db, &type_id, "Mira", "Cartographer"))?;
    let second = w.transaction(|db| entities::create_in(db, &type_id, "서연", "항해사"))?;
    entities::update(
        &w,
        &first,
        &json!({"aliases":["The Pilot"],"tags":["main cast"],"summary":"Coastal navigator"}),
    )?;
    let entry = entities::get(&w, &first)?;
    let document = entry["document"]["id"].as_str().ok_or("Missing document")?;
    let body =
        format!("# A journey\n\nMira visited [서연](entity://{second}).\n\n古都 서울의 이야기");
    w.transaction(|db| documents::save_in(&w, db, document, &body))?;
    assert_eq!(search::query(&w, "navigator", 20)?.len(), 1);
    assert_eq!(search::query(&w, "古都", 20)?.len(), 1);
    assert_eq!(
        search::query(&w, "type:Character tag:\"main cast\" journey", 20)?.len(),
        1
    );
    assert_eq!(
        entities::get(&w, &second)?["backlinks"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let first_revision: String = w.db.query_row(
        "SELECT id FROM revision WHERE document_id=?1",
        [document],
        |r| r.get(0),
    )?;
    assert_eq!(
        w.db.query_row(
            "SELECT body FROM revision WHERE id=?1",
            [first_revision],
            |r| r.get::<_, String>(0)
        )?,
        ""
    );
    let filepath = entry["document"]["path"].as_str().unwrap();
    trash::delete(&w, "entity", "entity", &first)?;
    assert!(!path.join(filepath).exists());
    assert!(search::query(&w, "navigator", 20)?.is_empty());
    let deleted: String = w.db.query_row("SELECT id FROM trash", [], |r| r.get(0))?;
    trash::restore(&w, &deleted)?;
    assert_eq!(entities::get(&w, &first)?["body"], body);
    assert_eq!(search::query(&w, "navigator", 20)?.len(), 1);
    drop(w);
    let reopened = World::open(&path)?;
    assert_eq!(entities::get(&reopened, &first)?["body"], body);
    assert_eq!(
        entities::get(&reopened, &first)?["tags"],
        json!(["main cast"])
    );
    Ok(())
}

#[test]
fn committed_file_write_is_recovered_after_interruption() -> world::Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("Recovery");
    let w = create::create(&path, "Recovery")?;
    // Simulate a crash between a successful SQLite commit and the file replacement.
    let tx = w.db.unchecked_transaction()?;
    World::queue_file(&tx, "content/recovered.md", b"committed body")?;
    tx.commit()?;
    assert!(!path.join("content/recovered.md").exists());
    drop(w);
    let reopened = World::open(&path)?;
    assert_eq!(
        fs::read_to_string(path.join("content/recovered.md"))?,
        "committed body"
    );
    assert_eq!(
        reopened
            .db
            .query_row("SELECT count(*) FROM __local_realm_files", [], |r| r
                .get::<_, i64>(0))?,
        0
    );
    assert!(reopened.output_file("../outside.md").is_err());
    Ok(())
}

#[test]
fn calendar_matches_reference_and_round_trips_negative_years() -> world::Result<()> {
    let definition: Value = serde_json::from_str(include_str!("../resources/calendar.json"))?;
    let calendar = calendar::Calendar::new(&definition)?;
    assert_eq!(
        calendar.minute(&json!({"year":980,"month":3,"day":1}))?,
        515517120
    );
    assert_eq!(
        calendar.minute(&json!({"year":1204,"month":3,"day":1}))?,
        633329280
    );
    assert!(calendar
        .minute(&json!({"year":1900,"month":2,"day":29}))
        .is_err());
    assert!(calendar
        .minute(&json!({"year":2000,"month":2,"day":29}))
        .is_ok());
    for year in [
        -1000000, -400, -101, -4, -1, 0, 1, 4, 100, 400, 980, 2000, 1000000,
    ] {
        for month in 1..=12 {
            let date = json!({"year":year,"month":month,"day":1,"hour":23,"minute":59});
            assert_eq!(calendar.date(calendar.minute(&date)?)?, date);
        }
    }
    let eras = calendar.eras();
    assert_eq!(eras[0]["boundary"], 1);
    assert_eq!(
        calendar.format(&json!({"year":0,"month":1,"day":1,"precision":"year"}))?,
        "1 BCE"
    );
    Ok(())
}

#[test]
fn map_and_timeline_restore_keep_links_and_binary_images() -> world::Result<()> {
    let root = tempfile::tempdir()?;
    let w = create::create(&root.path().join("Maps"), "Maps")?;
    let first = terrain::create(
        &w,
        "Northern coast",
        &json!({"cols":32,"rows":16,"seed":42,"fill":"continents"}),
    )?;
    let id = first["id"].as_str().ok_or("Missing map")?;
    let second = terrain::create(
        &w,
        "Southern coast",
        &json!({"cols":32,"rows":16,"seed":42,"fill":"continents"}),
    )?;
    assert_eq!(
        grid::read(&w, id)?.cells,
        grid::read(&w, second["id"].as_str().unwrap())?.cells
    );
    assert_eq!(terrain::relief(&w, id)?.len(), 512);
    let wrong_layer = second["layers"][0]["id"].clone();
    assert!(maps::feature_create(
        &w,
        "map_marker",
        id,
        &json!({"x":0.5,"y":0.5,"layer_id":wrong_layer})
    )
    .is_err());
    let pin = maps::feature_create(
        &w,
        "map_marker",
        id,
        &json!({"x":0.5,"y":0.5,"label":"Harbor"}),
    )?;
    let timeline = w.transaction(|db| timeline::create_in(db, "History", None))?;
    let event = timeline::save_event(
        &w,
        &timeline,
        None,
        &json!({"title":"Arrival","kind":"event","start_date":{"year":980,"month":3,"day":1},"marker_id":pin["id"]}),
    )?;
    assert_eq!(event["place"]["map_id"], id);
    let event_id = event["id"].as_str().unwrap();
    assert!(timeline::save_event(
        &w,
        &timeline,
        Some(event_id),
        &json!({"kind":"period","end_date":{"year":979,"month":1,"day":1}})
    )
    .is_err());
    assert_eq!(
        database::one(&w.db, "SELECT * FROM timeline_event WHERE id=?1", event_id)?["kind"],
        "event"
    );
    let before = fs::read(w.local_file(&format!("maps/{id}.png"))?)?;
    trash::delete(&w, "map", "map", id)?;
    assert!(!w.root.join(format!("maps/{id}.png")).exists());
    assert!(
        database::one(&w.db, "SELECT * FROM timeline_event WHERE id=?1", event_id)?["marker_id"]
            .is_null()
    );
    let deleted: String =
        w.db.query_row("SELECT id FROM trash WHERE ref_id=?1", [id], |r| r.get(0))?;
    trash::restore(&w, &deleted)?;
    assert_eq!(fs::read(w.local_file(&format!("maps/{id}.png"))?)?, before);
    assert_eq!(
        database::one(&w.db, "SELECT * FROM timeline_event WHERE id=?1", event_id)?["marker_id"],
        pin["id"]
    );
    assert_eq!(
        timeline::get(&w, &timeline, &json!({}))?["span"],
        json!([515517120, 515517120])
    );
    Ok(())
}

#[test]
fn manuscript_trash_restores_chapters_and_owned_documents() -> world::Result<()> {
    let root = tempfile::tempdir()?;
    let w = create::create(&root.path().join("Book"), "Book")?;
    w.transaction(|db| {
        db.execute(
            "INSERT INTO manuscript(id,name,created_at,updated_at) VALUES('book','Draft','','')",
            [],
        )?;
        Ok(())
    })?;
    let chapter =
        w.transaction(|db| manuscripts::chapter_create_in(db, "book", None, "Opening"))?;
    let document: String = w.db.query_row(
        "SELECT document_id FROM chapter WHERE id=?1",
        [chapter],
        |r| r.get(0),
    )?;
    w.transaction(|db| documents::save_in(&w, db, &document, "A chapter survives deletion."))?;
    trash::delete(&w, "manuscript", "manuscript", "book")?;
    assert!(w.document(&document).is_err());
    let deleted: String = w.db.query_row("SELECT id FROM trash", [], |r| r.get(0))?;
    trash::restore(&w, &deleted)?;
    assert_eq!(
        w.document(&document)?["body"],
        "A chapter survives deletion."
    );
    assert_eq!(manuscripts::get(&w, "book")?["chapter_count"], 1);
    assert_eq!(
        w.db.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r
            .get::<_, i64>(
            0
        ))?,
        0
    );
    Ok(())
}

#[test]
fn snapshot_restore_is_consistent_and_rejects_traversal() -> world::Result<()> {
    use std::io::Write;
    let temp = tempfile::tempdir()?;
    let w = create::create(&temp.path().join("Source"), "Source")?;
    let note = w.transaction(|db| documents::create_in(db, "note", "Important note", None))?;
    let id = note["id"].as_str().unwrap();
    w.transaction(|db| documents::save_in(&w, db, id, "Before snapshot."))?;
    let archive = temp.path().join("snapshot.zip");
    archives::snapshot(&w, &archive)?;
    w.transaction(|db| documents::save_in(&w, db, id, "After snapshot."))?;
    let dest = temp.path().join("Restored");
    fs::create_dir(&dest)?;
    archives::restore(&archive, &dest)?;
    let copy = World::open(&dest)?;
    assert_eq!(copy.document(id)?["body"], "Before snapshot.");
    assert!(archives::restore(&archive, &dest).is_err());
    let malicious = temp.path().join("malicious.zip");
    let mut writer = zip::ZipWriter::new(fs::File::create(&malicious)?);
    writer.start_file("../outside.txt", zip::write::SimpleFileOptions::default())?;
    writer.write_all(b"must not escape")?;
    writer.finish()?;
    let empty = temp.path().join("Empty");
    fs::create_dir(&empty)?;
    assert!(archives::restore(&malicious, &empty).is_err());
    assert!(fs::read_dir(&empty)?.next().is_none());
    assert!(!temp.path().join("outside.txt").exists());
    Ok(())
}

#[test]
fn graph_traverses_cycles_and_tables_filter_numeric_values() -> world::Result<()> {
    let root = tempfile::tempdir()?;
    let w = create::create(&root.path().join("Views"), "Views")?;
    let kind: String = w.db.query_row(
        "SELECT id FROM entity_type WHERE name='Character'",
        [],
        |r| r.get(0),
    )?;
    let a = w.transaction(|db| entities::create_in(db, &kind, "Ada", ""))?;
    let b = w.transaction(|db| entities::create_in(db, &kind, "Bea", ""))?;
    let c = w.transaction(|db| entities::create_in(db, &kind, "Cai", ""))?;
    w.transaction(|db| {
        for (a,b) in [(&a,&b),(&b,&c),(&c,&a)] {
            db.execute("INSERT INTO relationship(id,source_id,target_id,kind,created_at) VALUES(?1,?2,?3,'KNOWS','')",rusqlite::params![database::new_id(),a,b])?;
        }
        db.execute("INSERT INTO property_definition(id,type_id,key,label,data_type,created_at,updated_at) VALUES('test-age',?1,'test_age','Age','integer','','')",[&kind])?;
        Ok(())
    })?;
    entities::update(&w, &a, &json!({"properties":{"test_age":19}}))?;
    entities::update(&w, &b, &json!({"properties":{"test_age":27}}))?;
    entities::update(&w, &c, &json!({"properties":{"test_age":31}}))?;
    let graph = graph::build(
        &w,
        &json!({"focus":a,"depth":0,"include_properties":false,"include_articles":false}),
    )?;
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 3);
    assert_eq!(graph["edges"].as_array().unwrap().len(), 3);
    let table = tables::run(
        &w,
        Some(&kind),
        &json!({"columns":["name","test_age"],"filters":[{"key":"test_age","op":"gte","value":"27"}],"sort":{"key":"test_age","direction":"desc"}}),
    )?;
    assert_eq!(table["total"], 2);
    assert_eq!(table["rows"][0]["name"], "Cai");
    assert_eq!(table["totals"]["test_age"], 58.0);
    Ok(())
}

#[test]
fn json_bundle_and_document_import_preserve_text_and_links() -> world::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = create::create(&temp.path().join("Source"), "Source")?;
    let kind: String = source.db.query_row(
        "SELECT id FROM entity_type WHERE name='Character'",
        [],
        |r| r.get(0),
    )?;
    let a = source.transaction(|db| entities::create_in(db, &kind, "서연", "항해사"))?;
    let b = source.transaction(|db| entities::create_in(db, &kind, "Mira", "Cartographer"))?;
    let attachment_path = temp.path().join("route.txt");
    fs::write(&attachment_path, "항로 자료")?;
    let attachment = media::import(&source, &attachment_path, "항로")?;
    source.transaction(|db| {
        db.execute(
            "INSERT INTO entity_media(entity_id,media_id) VALUES(?1,?2)",
            rusqlite::params![a, attachment["id"].as_str()],
        )?;
        Ok(())
    })?;
    let document = entities::get(&source, &a)?["document"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    source.transaction(|db| {
        documents::save_in(
            &source,
            db,
            &document,
            &format!("[Mira](entity://{b})와 여행을 떠났다."),
        )
    })?;
    let data = export::bundle(&source)?;
    let destination = create::create(&temp.path().join("Destination"), "Destination")?;
    assert_eq!(export::import_bundle(&destination, &data)?["entities"], 2);
    assert_eq!(export::import_bundle(&destination, &data)?["entities"], 0);
    assert_eq!(
        destination
            .db
            .query_row("SELECT count(*) FROM media", [], |r| r.get::<_, i64>(0))?,
        1
    );
    assert_eq!(
        destination
            .db
            .query_row("SELECT count(*) FROM entity_media", [], |r| r
                .get::<_, i64>(0))?,
        1
    );
    let markdown_dir = temp.path().join("Markdown");
    export::export(
        &source,
        &json!({"format":"markdown","destination":markdown_dir}),
    )?;
    let markdown = fs::read_to_string(markdown_dir.join(format!("서연-{a}.md")))?;
    assert!(!markdown.contains("entity://"));
    let cycle = json!({"types":[{"id":"x","name":"New X","parent_id":"y"},{"id":"y","name":"New Y","parent_id":"x"}],"entities":[]});
    assert!(export::import_bundle(&destination, &cycle).is_err());
    assert_eq!(
        destination.db.query_row(
            "SELECT count(*) FROM entity_type WHERE name='New X'",
            [],
            |r| r.get::<_, i64>(0)
        )?,
        0
    );
    let imported: String =
        destination
            .db
            .query_row("SELECT id FROM entity WHERE name='Mira'", [], |r| r.get(0))?;
    assert_eq!(
        entities::get(&destination, &imported)?["backlinks"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let vault = temp.path().join("Vault");
    fs::create_dir(&vault)?;
    fs::write(vault.join("Harbor.md"),"---\ntype: Place\npopulation: 1200\ntags: [coast]\n---\n# Harbor\n\nA harbor visited by [[서연]].")?;
    let options = json!({"kind":"obsidian","paths":[vault],"target":"entries","split":0});
    let preview = import::scan(&destination, &options)?;
    assert_eq!(preview["notes"], 1);
    let choices = json!({"existing":"keep","missing":"text","groups":[{"key":"Place","new_type":"Place","fields":[{"label":"population","kind":"integer"}]}]});
    let result = import::run(&destination, &options, &choices)?;
    assert_eq!(result["created"], 1);
    assert_eq!(result["links"], 1);
    let harbor: String =
        destination
            .db
            .query_row("SELECT id FROM entity WHERE name='Harbor'", [], |r| {
                r.get(0)
            })?;
    assert_eq!(
        entities::get(&destination, &harbor)?["tags"],
        json!(["coast"])
    );
    let table = tables::run(
        &destination,
        None,
        &json!({"columns":["name","population"],"filters":[{"key":"population","op":"gte","value":1000}]}),
    )?;
    assert_eq!(table["total"], 1);
    Ok(())
}

#[test]
fn manuscript_outputs_are_readable_and_pdf_keeps_korean_text() -> world::Result<()> {
    let temp = tempfile::tempdir()?;
    let w = create::create(&temp.path().join("Manuscript"), "Manuscript")?;
    w.transaction(|db|{db.execute("INSERT INTO manuscript(id,name,created_at,updated_at) VALUES('book','항해 일지','','')",[])?;Ok(())})?;
    let chapter =
        w.transaction(|db| manuscripts::chapter_create_in(db, "book", None, "첫 항해"))?;
    let doc: String = w.db.query_row(
        "SELECT document_id FROM chapter WHERE id=?1",
        [&chapter],
        |r| r.get(0),
    )?;
    w.transaction(|db| {
        documents::save_in(
            &w,
            db,
            &doc,
            "바다로 떠났다. **Bold words** and *quiet thoughts*.\n\nA second paragraph.",
        )
    })?;
    for format in ["markdown", "docx", "pdf", "epub"] {
        let output = compile::compile(&w, "book", format, temp.path(), Some("times"))?;
        let path = Path::new(output["path"].as_str().unwrap());
        assert!(path.metadata()?.len() > 50);
        if let Some(directory) = std::env::var_os("LOCAL_REALM_TEST_ARTIFACTS") {
            let directory = PathBuf::from(directory);
            fs::create_dir_all(&directory)?;
            fs::copy(
                path,
                directory.join(path.file_name().ok_or("Missing output name")?),
            )?;
        }
        match format {
            "docx" => {
                let text = import::docx_markdown(path)?;
                assert!(text.contains("바다로 떠났다."));
                assert!(text.contains("Bold words"));
            }
            "epub" => {
                let mut zip = zip::ZipArchive::new(fs::File::open(path)?)?;
                assert_eq!(zip.by_index(0)?.name(), "mimetype");
                for i in 0..zip.len() {
                    let mut file = zip.by_index(i)?;
                    if file.name().ends_with(".xhtml")
                        || file.name().ends_with(".opf")
                        || file.name().ends_with(".xml")
                    {
                        use std::io::Read;
                        let mut xml = String::new();
                        file.read_to_string(&mut xml)?;
                        let mut reader = quick_xml::Reader::from_str(&xml);
                        loop {
                            if matches!(reader.read_event()?, quick_xml::events::Event::Eof) {
                                break;
                            }
                        }
                    }
                }
            }
            "pdf" => {
                let pdf = lopdf::Document::load(path)?;
                assert_eq!(
                    pdf.get_pages().len(),
                    compile::pages(&w, "book")?["total"].as_u64().unwrap() as usize
                );
                let text = pdf.extract_text(&[1, 2])?;
                assert!(text.contains("바다로 떠났다."), "{text}");
            }
            _ => {}
        }
    }
    Ok(())
}
