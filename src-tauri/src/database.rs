use crate::world::{rows, Result};
use rusqlite::{params_from_iter, types::Value as SqlValue, Connection};
use serde_json::Value;

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn required_text<'a>(input: &'a Value, key: &str) -> Result<&'a str> {
    input[key]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("{key} must contain text").into())
}

pub fn one(db: &Connection, sql: &str, id: &str) -> Result<Value> {
    rows(db, sql, [id])?
        .pop()
        .ok_or_else(|| format!("Record {id} not found").into())
}

fn sql_value(value: &Value) -> SqlValue {
    match value {
        Value::Null => SqlValue::Null,
        Value::Bool(value) => SqlValue::Integer(i64::from(*value)),
        Value::Number(value) => value
            .as_i64()
            .map(SqlValue::Integer)
            .unwrap_or_else(|| SqlValue::Real(value.as_f64().unwrap_or_default())),
        Value::String(value) => SqlValue::Text(value.clone()),
        _ => SqlValue::Text(value.to_string()),
    }
}

/// Table and column identifiers come exclusively from the domain modules.
pub fn update_fields(
    db: &Connection,
    table: &str,
    id: &str,
    input: &Value,
    allowed: &[&str],
    timestamp: bool,
) -> Result<()> {
    let object = input.as_object().ok_or("Expected an object")?;
    let mut assignments = Vec::new();
    let mut values = Vec::new();
    for column in allowed {
        if let Some(value) = object.get(*column) {
            assignments.push(format!("{column}=?"));
            values.push(sql_value(value));
        }
    }
    if assignments.is_empty() {
        return Ok(());
    }
    if timestamp {
        assignments.push("updated_at=?".into());
        values.push(SqlValue::Text(now()));
    }
    values.push(SqlValue::Text(id.into()));
    let sql = format!("UPDATE {table} SET {} WHERE id=?", assignments.join(", "));
    if db.execute(&sql, params_from_iter(values))? == 0 {
        return Err(format!("Record {id} not found").into());
    }
    Ok(())
}

pub fn reorder(db: &Connection, table: &str, ids: &[String]) -> Result<()> {
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    if unique.len() != ids.len() {
        return Err("Repeated item in ordering".into());
    }
    let mut query = db.prepare(&format!("UPDATE {table} SET sort_order=? WHERE id=?"))?;
    for (position, id) in ids.iter().enumerate() {
        if query.execute(rusqlite::params![position as i64, id])? != 1 {
            return Err(format!("Record {id} not found").into());
        }
    }
    Ok(())
}
