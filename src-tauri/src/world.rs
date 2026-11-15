use rusqlite::{types::ValueRef, Connection, OpenFlags, Params};
use serde_json::{json, Map, Value};
use std::{
    error::Error,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

pub struct World {
    pub root: PathBuf,
    pub db: Connection,
}

pub fn rows(db: &Connection, query: &str, args: impl Params) -> Result<Vec<Value>> {
    let mut statement = db.prepare_cached(query)?;
    let names: Vec<String> = statement
        .column_names()
        .into_iter()
        .map(String::from)
        .collect();
    let mut cursor = statement.query(args)?;
    let mut result = Vec::new();
    while let Some(row) = cursor.next()? {
        let mut object = Map::new();
        for (i, name) in names.iter().enumerate() {
            let value = match row.get_ref(i)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(n)
                    if matches!(
                        name.as_str(),
                        "required" | "locked" | "visible" | "flip" | "symmetrical" | "is_default"
                    ) =>
                {
                    json!(n != 0)
                }
                ValueRef::Integer(n) => json!(n),
                ValueRef::Real(n) => json!(n),
                ValueRef::Text(bytes) => {
                    let text = std::str::from_utf8(bytes)?;
                    if matches!(
                        name.as_str(),
                        "config"
                            | "definition"
                            | "layout"
                            | "ebook"
                            | "image_frame"
                            | "points"
                            | "kinds"
                            | "tags"
                            | "aliases"
                            | "date"
                            | "valid_from"
                            | "valid_until"
                            | "value"
                            | "start_date"
                            | "end_date"
                    ) {
                        if text.is_empty() {
                            Value::Null
                        } else {
                            serde_json::from_str(text)?
                        }
                    } else {
                        json!(text)
                    }
                }
                ValueRef::Blob(_) => return Err(format!("Unexpected binary column {name}").into()),
            };
            object.insert(name.clone(), value);
        }
        result.push(Value::Object(object));
    }
    Ok(result)
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut temp = tempfile::NamedTempFile::new_in(path.parent().ok_or("Missing parent folder")?)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    Ok(())
}

impl World {
    pub fn open(path: &Path) -> Result<Self> {
        let root = path.canonicalize()?;
        let manifest: Value = serde_json::from_slice(&fs::read(root.join("world.json"))?)?;
        if !manifest["id"].is_string() || !manifest["name"].is_string() {
            return Err("Missing realm identity".into());
        }
        let mut db =
            Connection::open_with_flags(root.join("world.db"), OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        db.busy_timeout(std::time::Duration::from_secs(10))?;
        db.pragma_update(None, "foreign_keys", true)?;
        let version: usize = db.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version == 0 || version > crate::migrations::ALL.len() {
            return Err(format!("Unsupported realm schema version {version}").into());
        }
        if version < crate::migrations::ALL.len() {
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            for migration in &crate::migrations::ALL[version..] {
                tx.execute_batch(migration)?;
            }
            tx.pragma_update(None, "user_version", crate::migrations::ALL.len())?;
            tx.commit()?;
        }
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS __local_realm_files (
            path TEXT PRIMARY KEY, contents BLOB NOT NULL, remove_file INTEGER NOT NULL DEFAULT 0
        )",
        )?;
        let has_remove: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('__local_realm_files') WHERE name='remove_file')",
            [], |r| r.get(0),
        )?;
        if !has_remove {
            db.execute_batch(
                "ALTER TABLE __local_realm_files ADD COLUMN remove_file INTEGER NOT NULL DEFAULT 0",
            )?;
        }
        let world = Self { root, db };
        world.flush_files()?;
        if manifest["schema_version"].as_u64() != Some(crate::migrations::ALL.len() as u64) {
            let mut manifest = manifest;
            manifest["schema_version"] = json!(crate::migrations::ALL.len());
            write_atomic(
                &world.root.join("world.json"),
                &serde_json::to_vec_pretty(&manifest)?,
            )?;
        }
        let pending: bool = world.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM pending_work WHERE task='text')",
            [],
            |r| r.get(0),
        )?;
        if pending {
            crate::search::rebuild(&world)?;
        }
        Ok(world)
    }

    pub fn info(&self) -> Result<Value> {
        let mut info: Value = serde_json::from_slice(&fs::read(self.root.join("world.json"))?)?;
        info["path"] = json!(dunce::simplified(&self.root));
        info["stats"] = self.stats()?;
        Ok(info)
    }

    pub fn stats(&self) -> Result<Value> {
        Ok(rows(&self.db, "SELECT (SELECT COUNT(*) FROM entity) AS entities,
            (SELECT COUNT(*) FROM entity_type) AS types, (SELECT COUNT(*) FROM relationship) AS relationships,
            (SELECT COUNT(*) FROM document) AS documents, (SELECT COUNT(*) FROM media) AS media,
            (SELECT coalesce(sum(word_count),0) FROM document) AS words", [])?.remove(0))
    }

    pub fn entities(&self, filter: &Value) -> Result<Vec<Value>> {
        let order = match filter["sort"].as_str() {
            Some("order") => "e.sort_order, e.name COLLATE NOCASE, e.id",
            Some("recent") => "e.updated_at DESC, e.id",
            _ => "e.name COLLATE NOCASE, e.id",
        };
        let sql = format!("SELECT e.*, t.name AS type_name, t.icon AS type_icon, t.color AS type_color,m.path AS image_path,
            (SELECT json_group_array(name) FROM (SELECT tag.name FROM tag
              JOIN entity_tag et ON et.tag_id=tag.id WHERE et.entity_id=e.id ORDER BY tag.name COLLATE NOCASE)) AS tags,
            (SELECT json_group_array(name) FROM (SELECT name FROM alias WHERE entity_id=e.id ORDER BY name COLLATE NOCASE)) AS aliases
            FROM entity e JOIN entity_type t ON t.id=e.type_id LEFT JOIN media m ON m.id=e.image_media_id
            WHERE (?1 IS NULL OR e.type_id=?1) ORDER BY {order} LIMIT ?2");
        rows(
            &self.db,
            &sql,
            rusqlite::params![
                filter["type_id"].as_str(),
                filter["limit"].as_i64().unwrap_or(5000).max(0)
            ],
        )
    }

    pub fn transaction<T>(&self, action: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        self.flush_files()?;
        let tx = rusqlite::Transaction::new_unchecked(
            &self.db,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let result = action(&tx)?;
        tx.commit()?;
        self.flush_files()?;
        Ok(result)
    }

    pub fn queue_file(db: &Connection, path: &str, contents: &[u8]) -> Result<()> {
        db.execute(
            "INSERT INTO __local_realm_files(path,contents) VALUES (?1,?2)
            ON CONFLICT(path) DO UPDATE SET contents=excluded.contents,remove_file=0",
            rusqlite::params![path, contents],
        )?;
        Ok(())
    }

    pub fn queue_remove(db: &Connection, path: &str) -> Result<()> {
        db.execute(
            "INSERT INTO __local_realm_files(path,contents,remove_file) VALUES (?1,x'',1)
            ON CONFLICT(path) DO UPDATE SET contents=x'',remove_file=1",
            [path],
        )?;
        Ok(())
    }

    pub fn flush_files(&self) -> Result<()> {
        let pending: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM __local_realm_files)",
            [],
            |r| r.get(0),
        )?;
        if !pending {
            return Ok(());
        }
        let tx = rusqlite::Transaction::new_unchecked(
            &self.db,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        {
            let mut statement = tx.prepare(
                "SELECT path,contents,remove_file FROM __local_realm_files ORDER BY path",
            )?;
            let pending = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            })?;
            for file in pending {
                let (relative, bytes, remove_file) = file?;
                let path = self.output_file(&relative)?;
                if remove_file {
                    match fs::remove_file(&path) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error.into()),
                    }
                } else {
                    write_atomic(&path, &bytes)?;
                }
            }
        }
        tx.execute("DELETE FROM __local_realm_files", [])?;
        tx.commit()?;
        Ok(())
    }

    pub fn output_file(&self, relative: &str) -> Result<PathBuf> {
        let part = Path::new(relative);
        if part
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err("Expected a relative realm path".into());
        }
        let path = self.root.join(part);
        let mut existing = path.as_path();
        while !existing.exists() {
            existing = existing.parent().ok_or("Missing realm directory")?;
        }
        if !existing.canonicalize()?.starts_with(&self.root) {
            return Err("File outside the realm".into());
        }
        let parent = path.parent().ok_or("Missing file directory")?;
        fs::create_dir_all(parent)?;
        Ok(path)
    }

    pub fn local_file(&self, relative: &str) -> Result<PathBuf> {
        let path = self.root.join(relative).canonicalize()?;
        if !path.starts_with(&self.root) || !path.is_file() {
            return Err("File outside the realm".into());
        }
        Ok(path)
    }

    pub fn document(&self, id: &str) -> Result<Value> {
        let mut doc = rows(&self.db, "SELECT * FROM document WHERE id=?1", [id])?
            .pop()
            .ok_or("Document not found")?;
        use rusqlite::OptionalExtension;
        let relative = doc["path"].as_str().ok_or("Missing document path")?;
        let pending: Option<Vec<u8>> = self
            .db
            .query_row(
                "SELECT contents FROM __local_realm_files WHERE path=?1 AND remove_file=0",
                [relative],
                |row| row.get(0),
            )
            .optional()?;
        let raw = match pending {
            Some(bytes) => String::from_utf8(bytes)?,
            None => fs::read_to_string(self.local_file(relative)?)?,
        };
        let normalized = raw.replace("\r\n", "\n");
        let body = crate::documents::body_from_file(&normalized);
        doc["body"] = json!(body);
        Ok(doc)
    }
}
