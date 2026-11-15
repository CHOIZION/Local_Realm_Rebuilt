//! Calendar arithmetic uses astronomical years, including year zero and negative years.
use crate::{
    database::{new_id, now, one},
    world::{rows, Result, World},
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

pub struct Calendar {
    definition: Value,
    months: Vec<i64>,
    minutes_per_hour: i64,
    hours_per_day: i64,
}

impl Calendar {
    pub fn new(definition: &Value) -> Result<Self> {
        let months = definition["months"]
            .as_array()
            .filter(|m| !m.is_empty() && m.len() <= 1000)
            .ok_or("A calendar needs months")?;
        let mut lengths = Vec::new();
        for month in months {
            let days = month["days"]
                .as_i64()
                .filter(|d| (1..=10000).contains(d))
                .ok_or("Invalid month length")?;
            if month["name"].as_str().is_none_or(|s| s.trim().is_empty()) {
                return Err("A month needs a name".into());
            }
            lengths.push(days);
        }
        let minutes_per_hour = definition["minutes_per_hour"]
            .as_i64()
            .filter(|n| (1..=1000).contains(n))
            .ok_or("Invalid minutes per hour")?;
        let hours_per_day = definition["hours_per_day"]
            .as_i64()
            .filter(|n| (1..=1000).contains(n))
            .ok_or("Invalid hours per day")?;
        if !definition["leap"].is_null() {
            let leap = &definition["leap"];
            let index = leap["month_index"].as_u64().ok_or("Invalid leap month")? as usize;
            if index >= months.len()
                || leap["every"].as_i64().is_none_or(|v| v < 1)
                || leap["days"]
                    .as_i64()
                    .is_none_or(|v| !(0..=10000).contains(&v))
            {
                return Err("Invalid leap rule".into());
            }
            for key in ["except_every", "also_every"] {
                if !leap[key].is_null() && leap[key].as_i64().is_none_or(|v| v < 0) {
                    return Err("Invalid leap interval".into());
                }
            }
        }
        Ok(Self {
            definition: definition.clone(),
            months: lengths,
            minutes_per_hour,
            hours_per_day,
        })
    }
    fn leap(&self, year: i64) -> bool {
        let rule = &self.definition["leap"];
        let every = rule["every"].as_i64().unwrap_or(0);
        let except = rule["except_every"].as_i64().unwrap_or(0);
        let also = rule["also_every"].as_i64().unwrap_or(0);
        every > 0
            && year.rem_euclid(every) == 0
            && (except == 0
                || year.rem_euclid(except) != 0
                || (also > 0 && year.rem_euclid(also) == 0))
    }
    fn month_days(&self, year: i64, index: usize) -> i64 {
        self.months[index]
            + if self.definition["leap"]["month_index"].as_u64() == Some(index as u64)
                && self.leap(year)
            {
                self.definition["leap"]["days"].as_i64().unwrap_or(0)
            } else {
                0
            }
    }
    fn days_before_year(&self, year: i64) -> i64 {
        let count = |key: &str| {
            let interval = self.definition["leap"][key].as_i64().unwrap_or(0);
            if interval <= 0 {
                0
            } else if year > 0 {
                (year - 1) / interval + 1
            } else {
                -((-year) / interval)
            }
        };
        year * self.months.iter().sum::<i64>()
            + (count("every") - count("except_every") + count("also_every"))
                * self.definition["leap"]["days"].as_i64().unwrap_or(0)
    }
    pub fn minute(&self, date: &Value) -> Result<i64> {
        let year = date["year"]
            .as_i64()
            .filter(|y| (-1_000_000..=1_000_000).contains(y))
            .ok_or("Year is outside the supported range")?;
        let month = date["month"].as_u64().unwrap_or(1) as usize;
        if month == 0 || month > self.months.len() {
            return Err("Invalid month".into());
        }
        let day = date["day"].as_i64().unwrap_or(1);
        let hour = date["hour"].as_i64().unwrap_or(0);
        let minute = date["minute"].as_i64().unwrap_or(0);
        if day < 1
            || day > self.month_days(year, month - 1)
            || !(0..self.hours_per_day).contains(&hour)
            || !(0..self.minutes_per_hour).contains(&minute)
        {
            return Err("Invalid calendar date".into());
        }
        let days = self.days_before_year(year)
            + (0..month - 1)
                .map(|i| self.month_days(year, i))
                .sum::<i64>()
            + day
            - 1;
        days.checked_mul(self.hours_per_day)
            .and_then(|v| v.checked_add(hour))
            .and_then(|v| v.checked_mul(self.minutes_per_hour))
            .and_then(|v| v.checked_add(minute))
            .ok_or_else(|| "Date is too large".into())
    }
    pub fn date(&self, minute: i64) -> Result<Value> {
        let day_minutes = self.hours_per_day * self.minutes_per_hour;
        let days = minute.div_euclid(day_minutes);
        let (mut low, mut high) = (-1_000_000i64, 1_000_001i64);
        if days < self.days_before_year(low) || days >= self.days_before_year(high) {
            return Err("Date is outside the supported range".into());
        }
        while low + 1 < high {
            let middle = low + (high - low) / 2;
            if self.days_before_year(middle) <= days {
                low = middle;
            } else {
                high = middle;
            }
        }
        let mut remaining = days - self.days_before_year(low);
        let mut month = 0;
        while month + 1 < self.months.len() && remaining >= self.month_days(low, month) {
            remaining -= self.month_days(low, month);
            month += 1;
        }
        let in_day = minute.rem_euclid(day_minutes);
        Ok(
            json!({"year":low,"month":month+1,"day":remaining+1,"hour":in_day/self.minutes_per_hour,"minute":in_day%self.minutes_per_hour}),
        )
    }
    pub fn eras(&self) -> Vec<Value> {
        let mut eras = self.definition["eras"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        eras.sort_by_key(|e| e["start_year"].as_i64().unwrap_or(0));
        for i in 0..eras.len() {
            let boundary = eras
                .get(i + 1)
                .and_then(|e| e["start_year"].as_i64())
                .unwrap_or_else(|| eras[i]["start_year"].as_i64().unwrap_or(0) + 1);
            eras[i]["boundary"] = json!(boundary);
        }
        eras
    }
    pub fn format(&self, date: &Value) -> Result<String> {
        self.minute(date)?;
        let year = date["year"].as_i64().ok_or("Missing year")?;
        let eras = self.eras();
        let era = eras
            .iter()
            .rev()
            .find(|e| e["start_year"].as_i64().unwrap_or(0) <= year)
            .or_else(|| eras.first().filter(|e| e["counts_backward"] == true));
        let year_label = if let Some(era) = era {
            let shown = if era["counts_backward"] == true {
                era["boundary"].as_i64().unwrap_or(1) - year
            } else {
                year - era["start_year"].as_i64().unwrap_or(1) + 1
            };
            format!("{shown} {}", era["abbreviation"].as_str().unwrap_or(""))
                .trim()
                .to_owned()
        } else {
            year.to_string()
        };
        if date["precision"] == "year" {
            return Ok(year_label);
        }
        let month = date["month"].as_u64().unwrap_or(1) as usize - 1;
        let month_name = self.definition["months"][month]["name"]
            .as_str()
            .unwrap_or("");
        if date["precision"] == "month" {
            return Ok(format!("{month_name} {year_label}"));
        }
        let mut text = format!(
            "{} {month_name} {year_label}",
            date["day"].as_i64().unwrap_or(1)
        );
        if date["hour"].as_i64().unwrap_or(0) != 0
            || date["minute"].as_i64().unwrap_or(0) != 0
            || date["precision"] == "minute"
        {
            text.push_str(&format!(
                " {:02}:{:02}",
                date["hour"].as_i64().unwrap_or(0),
                date["minute"].as_i64().unwrap_or(0)
            ));
        }
        Ok(text)
    }
}

pub fn default_in(db: &Connection) -> Result<Value> {
    if let Some(calendar) = rows(
        db,
        "SELECT * FROM calendar ORDER BY is_default DESC,created_at,id LIMIT 1",
        [],
    )?
    .pop()
    {
        return Ok(calendar);
    }
    let id = new_id();
    let definition: Value = serde_json::from_str(include_str!("../resources/calendar.json"))?;
    db.execute("INSERT INTO calendar(id,name,definition,is_default,created_at,updated_at) VALUES(?1,'Standard calendar',?2,1,?3,?3)",params![id,definition.to_string(),now()])?;
    one(db, "SELECT * FROM calendar WHERE id=?1", &id)
}
pub fn resolve(db: &Connection, id: Option<&str>) -> Result<Value> {
    match id {
        Some(id) => one(db, "SELECT * FROM calendar WHERE id=?1", id),
        None => default_in(db),
    }
}
pub fn recalculate(db: &Connection) -> Result<()> {
    for timeline in rows(db, "SELECT id,calendar_id FROM timeline", [])? {
        let calendar = resolve(db, timeline["calendar_id"].as_str())?;
        let calendar = Calendar::new(&calendar["definition"])?;
        for event in rows(
            db,
            "SELECT id,start_date,end_date FROM timeline_event WHERE timeline_id=?1",
            [timeline["id"].as_str()],
        )? {
            let start = calendar.minute(&event["start_date"])?;
            let end = if event["end_date"].is_null() {
                None
            } else {
                Some(calendar.minute(&event["end_date"])?)
            };
            if end.is_some_and(|v| v < start) {
                return Err("An event ends before it starts".into());
            }
            db.execute(
                "UPDATE timeline_event SET start_minute=?1,end_minute=?2 WHERE id=?3",
                params![start, end, event["id"].as_str()],
            )?;
        }
    }
    Ok(())
}
#[tauri::command(async)]
pub fn calendar_default(state: tauri::State<crate::Session>) -> crate::Reply<Value> {
    state.read(|w| w.transaction(default_in))
}
#[tauri::command(async)]
pub fn calendar_create(
    state: tauri::State<crate::Session>,
    name: String,
    definition: Value,
) -> crate::Reply<Value> {
    state.read(|w|w.transaction(|db| {
        Calendar::new(&definition)?;
        if name.trim().is_empty(){return Err("Give the calendar a name".into());}
        let id=new_id();
        db.execute("INSERT INTO calendar(id,name,definition,is_default,created_at,updated_at) VALUES(?1,?2,?3,NOT EXISTS(SELECT 1 FROM calendar),?4,?4)",params![id,name.trim(),definition.to_string(),now()])?;
        one(db,"SELECT * FROM calendar WHERE id=?1",&id)
    }))
}
#[tauri::command(async)]
pub fn calendar_update(
    state: tauri::State<crate::Session>,
    id: String,
    name: String,
    definition: Value,
) -> crate::Reply<Value> {
    state.read(|w| {
        w.transaction(|db| {
            Calendar::new(&definition)?;
            if name.trim().is_empty() {
                return Err("Give the calendar a name".into());
            }
            db.execute(
                "UPDATE calendar SET name=?1,definition=?2,updated_at=?3 WHERE id=?4",
                params![name.trim(), definition.to_string(), now(), id],
            )?;
            recalculate(db)?;
            one(db, "SELECT * FROM calendar WHERE id=?1", &id)
        })
    })
}
#[tauri::command(async)]
pub fn calendar_delete(state: tauri::State<crate::Session>, id: String) -> crate::Reply<()> {
    state.read(|w| {
        w.transaction(|db| {
            let current = one(db, "SELECT * FROM calendar WHERE id=?1", &id)?;
            if current["is_default"] == true {
                return Err("The default calendar cannot be deleted".into());
            }
            db.execute("DELETE FROM calendar WHERE id=?1", [id])?;
            recalculate(db)
        })
    })
}
fn with_calendar<T>(
    w: &World,
    id: Option<&str>,
    f: impl FnOnce(Calendar) -> Result<T>,
) -> Result<T> {
    let record = w.transaction(|db| resolve(db, id))?;
    f(Calendar::new(&record["definition"])?)
}
#[tauri::command(async)]
pub fn calendar_eras(
    state: tauri::State<crate::Session>,
    calendar_id: Option<String>,
) -> crate::Reply<Vec<Value>> {
    state.read(|w| with_calendar(w, calendar_id.as_deref(), |c| Ok(c.eras())))
}
#[tauri::command(async)]
pub fn calendar_format(
    state: tauri::State<crate::Session>,
    calendar_id: Option<String>,
    date: Value,
) -> crate::Reply<String> {
    state.read(|w| with_calendar(w, calendar_id.as_deref(), |c| c.format(&date)))
}
#[tauri::command(async)]
pub fn timeline_ticks(
    state: tauri::State<crate::Session>,
    calendar_id: Option<String>,
    from: i64,
    to: i64,
    max: Option<usize>,
) -> crate::Reply<Vec<Value>> {
    state.read(|w|with_calendar(w,calendar_id.as_deref(),|calendar| {
        if from>=to{return Ok(vec![]);}
        let count=max.unwrap_or(14).clamp(2,100);
        let target=(to as f64-from as f64)/count as f64;
        let magnitude=10f64.powf(target.log10().floor());
        let step=([1.,2.,5.,10.].into_iter().find(|v|v*magnitude>=target).unwrap_or(10.)*magnitude).max(1.) as i64;
        let mut result=Vec::new();
        let mut minute=from.div_euclid(step).saturating_mul(step);
        if minute<from{minute=minute.saturating_add(step);}
        while minute<=to && result.len()<=count+1 {
            let date=calendar.date(minute)?;
            result.push(json!({"minute":minute,"label":calendar.format(&date)?,"major":date["month"]==1 && date["day"]==1,"date":date}));
            let Some(next)=minute.checked_add(step) else {break;}; minute=next;
        }
        Ok(result)
    }))
}
