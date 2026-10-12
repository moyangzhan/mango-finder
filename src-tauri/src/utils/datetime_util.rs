use crate::errors::AppError;
use crate::global::{DEFAULT_DATETIME_FORMAT, DEFAULT_DATETIME_MICRO_FORMAT};
use chrono::{DateTime, Local, NaiveDateTime};
use serde::{self, Deserialize, Deserializer, Serializer};

pub fn serialize<S>(date: &DateTime<Local>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let s = date.format(DEFAULT_DATETIME_FORMAT).to_string();
    serializer.serialize_str(&s)
}

/// Module for handling Option<DateTime<Local>> serialization
pub mod option {
    use super::*;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(date: &Option<DateTime<Local>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match date {
            Some(dt) => {
                let s = dt.format(DEFAULT_DATETIME_FORMAT).to_string();
                serializer.serialize_some(&s)
            }
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<DateTime<Local>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt: Option<String> = Option::deserialize(deserializer)?;
        match opt {
            Some(s) => super::str_to_datetime(&s)
                .map(Some)
                .map_err(serde::de::Error::custom),
            None => Ok(None),
        }
    }
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<DateTime<Local>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    str_to_datetime(s.as_str()).map_err(|e| serde::de::Error::custom(e.to_string()))
}

pub fn str_to_local_datetime(s: &str, fmt: &str) -> Result<DateTime<Local>, AppError> {
    if s.is_empty() {
        return Ok(DateTime::default());
    }
    match NaiveDateTime::parse_from_str(s, fmt) {
        Ok(naive) => match naive.and_local_timezone(Local).single() {
            Some(dt) => Ok(dt),
            None => Ok(DateTime::default()),
        },
        Err(_) => Ok(DateTime::default()),
    }
}

pub fn str_to_datetime(s: &str) -> Result<DateTime<Local>, AppError> {
    str_to_local_datetime(s, DEFAULT_DATETIME_FORMAT)
}

pub fn str_to_micro_datetime(s: &str) -> Result<DateTime<Local>, AppError> {
    str_to_local_datetime(s, DEFAULT_DATETIME_MICRO_FORMAT)
}

pub fn datetime_to_str(dt: &DateTime<Local>) -> String {
    dt.format(DEFAULT_DATETIME_FORMAT).to_string()
}

pub fn micro_datetime_to_str(dt: &DateTime<Local>) -> String {
    dt.format(DEFAULT_DATETIME_MICRO_FORMAT).to_string()
}
pub fn systemtime_to_datetime(time: std::time::SystemTime) -> DateTime<Local> {
    let datetime: DateTime<Local> = time.into();
    datetime
}

pub fn systemtime_to_str(time: std::time::SystemTime) -> String {
    let datetime: DateTime<Local> = systemtime_to_datetime(time);
    datetime_to_str(&datetime)
}

fn main() -> Result<(), AppError> {
    let dt = str_to_local_datetime("2024-05-20 14:30:00", "%Y-%m-%d %H:%M:%S")?;
    println!("local datetime: {}", dt);
    Ok(())
}
