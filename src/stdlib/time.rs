//! std/time module - Comprehensive time and date operations
//! 
//! Provides Go-like time handling with timezone support, date parsing,
//! arithmetic, and formatting.

use std::collections::HashMap;
use std::time::Duration;
use chrono::{DateTime, Utc, NaiveDateTime, Datelike, Timelike, TimeZone};
use chrono_tz::Tz;
use crate::interpreter::Value;
use crate::error::IntentError;

/// Helper to create a datetime map from chrono DateTime
fn datetime_to_map<T: TimeZone>(dt: &DateTime<T>, tz_name: &str) -> HashMap<String, Value> 
where T::Offset: std::fmt::Display {
    let mut map = HashMap::new();
    map.insert("year".to_string(), Value::Int(dt.year() as i64));
    map.insert("month".to_string(), Value::Int(dt.month() as i64));
    map.insert("day".to_string(), Value::Int(dt.day() as i64));
    map.insert("hour".to_string(), Value::Int(dt.hour() as i64));
    map.insert("minute".to_string(), Value::Int(dt.minute() as i64));
    map.insert("second".to_string(), Value::Int(dt.second() as i64));
    map.insert("nanosecond".to_string(), Value::Int(dt.nanosecond() as i64));
    map.insert("weekday".to_string(), Value::Int(dt.weekday().num_days_from_sunday() as i64));
    map.insert("day_of_year".to_string(), Value::Int(dt.ordinal() as i64));
    map.insert("timestamp".to_string(), Value::Int(dt.timestamp()));
    map.insert("timezone".to_string(), Value::String(tz_name.to_string()));
    map.insert("offset".to_string(), Value::String(dt.offset().to_string()));
    map
}

/// Parse timezone string to chrono_tz::Tz
fn parse_timezone(tz: &str) -> Result<Tz, IntentError> {
    tz.parse::<Tz>()
        .map_err(|_| IntentError::RuntimeError(format!("Invalid timezone: '{}'. Use IANA format like 'America/New_York'", tz)))
}

/// Initialize the std/time module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();
    
    // ========== Current Time ==========
    
    // now() -> Int - Current Unix timestamp in seconds (UTC)
    module.insert("now".to_string(), Value::NativeFunction {
        name: "now".to_string(),
        arity: 0,
        func: |_args| {
            Ok(Value::Int(Utc::now().timestamp()))
        },
    });
    
    // now_millis() -> Int - Current Unix timestamp in milliseconds
    module.insert("now_millis".to_string(), Value::NativeFunction {
        name: "now_millis".to_string(),
        arity: 0,
        func: |_args| {
            Ok(Value::Int(Utc::now().timestamp_millis()))
        },
    });
    
    // now_nanos() -> Int - Current Unix timestamp in nanoseconds
    module.insert("now_nanos".to_string(), Value::NativeFunction {
        name: "now_nanos".to_string(),
        arity: 0,
        func: |_args| {
            match Utc::now().timestamp_nanos_opt() {
                Some(nanos) => Ok(Value::Int(nanos)),
                None => Err(IntentError::RuntimeError("Timestamp out of range for nanoseconds".to_string())),
            }
        },
    });
    
    // ========== Timezone Conversion ==========
    
    // to_timezone(timestamp, timezone) -> Map - Convert timestamp to timezone
    // Returns { year, month, day, hour, minute, second, weekday, day_of_year, timestamp, timezone, offset }
    module.insert("to_timezone".to_string(), Value::NativeFunction {
        name: "to_timezone".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::Int(ts), Value::String(tz_str)) => {
                    let tz = parse_timezone(tz_str)?;
                    let dt = DateTime::from_timestamp(*ts, 0)
                        .ok_or_else(|| IntentError::RuntimeError("Invalid timestamp".to_string()))?
                        .with_timezone(&tz);
                    Ok(Value::Map(datetime_to_map(&dt, tz_str)))
                }
                _ => Err(IntentError::TypeError("to_timezone() requires (timestamp: Int, timezone: String)".to_string())),
            }
        },
    });
    
    // to_utc(timestamp) -> Map - Get UTC datetime parts from timestamp
    module.insert("to_utc".to_string(), Value::NativeFunction {
        name: "to_utc".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::Int(ts) => {
                    let dt = DateTime::from_timestamp(*ts, 0)
                        .ok_or_else(|| IntentError::RuntimeError("Invalid timestamp".to_string()))?;
                    Ok(Value::Map(datetime_to_map(&dt, "UTC")))
                }
                _ => Err(IntentError::TypeError("to_utc() requires a timestamp".to_string())),
            }
        },
    });
    
    // ========== Formatting ==========
    
    // format(timestamp, format_str) -> String - Format timestamp as UTC
    // Supports: %Y %m %d %H %M %S %f %Z %z %a %A %b %B %j %U %W %w
    module.insert("format".to_string(), Value::NativeFunction {
        name: "format".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::Int(ts), Value::String(fmt)) => {
                    let dt = DateTime::from_timestamp(*ts, 0)
                        .ok_or_else(|| IntentError::RuntimeError("Invalid timestamp".to_string()))?;
                    Ok(Value::String(dt.format(fmt).to_string()))
                }
                _ => Err(IntentError::TypeError("format() requires (timestamp: Int, format: String)".to_string())),
            }
        },
    });
    
    // format_in(timestamp, timezone, format_str) -> String - Format with timezone
    module.insert("format_in".to_string(), Value::NativeFunction {
        name: "format_in".to_string(),
        arity: 3,
        func: |args| {
            match (&args[0], &args[1], &args[2]) {
                (Value::Int(ts), Value::String(tz_str), Value::String(fmt)) => {
                    let tz = parse_timezone(tz_str)?;
                    let dt = DateTime::from_timestamp(*ts, 0)
                        .ok_or_else(|| IntentError::RuntimeError("Invalid timestamp".to_string()))?
                        .with_timezone(&tz);
                    Ok(Value::String(dt.format(fmt).to_string()))
                }
                _ => Err(IntentError::TypeError("format_in() requires (timestamp: Int, timezone: String, format: String)".to_string())),
            }
        },
    });
    
    // to_iso(timestamp) -> String - Format as ISO 8601 (RFC 3339)
    module.insert("to_iso".to_string(), Value::NativeFunction {
        name: "to_iso".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::Int(ts) => {
                    let dt = DateTime::from_timestamp(*ts, 0)
                        .ok_or_else(|| IntentError::RuntimeError("Invalid timestamp".to_string()))?;
                    Ok(Value::String(dt.to_rfc3339()))
                }
                _ => Err(IntentError::TypeError("to_iso() requires a timestamp".to_string())),
            }
        },
    });
    
    // ========== Parsing ==========
    
    // parse(date_str, format_str) -> Result<Int, String> - Parse string to timestamp
    module.insert("parse".to_string(), Value::NativeFunction {
        name: "parse".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::String(date_str), Value::String(fmt)) => {
                    match NaiveDateTime::parse_from_str(date_str, fmt) {
                        Ok(naive) => {
                            let dt = naive.and_utc();
                            Ok(Value::EnumValue {
                                enum_name: "Result".to_string(),
                                variant: "Ok".to_string(),
                                values: vec![Value::Int(dt.timestamp())],
                            })
                        }
                        Err(e) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(format!("Parse error: {}", e))],
                        }),
                    }
                }
                _ => Err(IntentError::TypeError("parse() requires (date_str: String, format: String)".to_string())),
            }
        },
    });
    
    // parse_iso(iso_str) -> Result<Int, String> - Parse ISO 8601 / RFC 3339 string
    module.insert("parse_iso".to_string(), Value::NativeFunction {
        name: "parse_iso".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::String(s) => {
                    match DateTime::parse_from_rfc3339(s) {
                        Ok(dt) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Ok".to_string(),
                            values: vec![Value::Int(dt.timestamp())],
                        }),
                        Err(e) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(format!("Parse error: {}", e))],
                        }),
                    }
                }
                _ => Err(IntentError::TypeError("parse_iso() requires a string".to_string())),
            }
        },
    });
    
    // ========== Date Creation ==========
    
    // make_time(year, month, day, hour, minute, second) -> Result<Int, String>
    // Create timestamp from components (assumes UTC)
    module.insert("make_time".to_string(), Value::NativeFunction {
        name: "make_time".to_string(),
        arity: 6,
        func: |args| {
            match (&args[0], &args[1], &args[2], &args[3], &args[4], &args[5]) {
                (Value::Int(year), Value::Int(month), Value::Int(day), 
                 Value::Int(hour), Value::Int(minute), Value::Int(second)) => {
                    match Utc.with_ymd_and_hms(
                        *year as i32, *month as u32, *day as u32,
                        *hour as u32, *minute as u32, *second as u32
                    ) {
                        chrono::LocalResult::Single(dt) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Ok".to_string(),
                            values: vec![Value::Int(dt.timestamp())],
                        }),
                        _ => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String("Invalid date/time components".to_string())],
                        }),
                    }
                }
                _ => Err(IntentError::TypeError("make_time() requires 6 integers: (year, month, day, hour, minute, second)".to_string())),
            }
        },
    });
    
    // make_date(year, month, day) -> Result<Int, String>
    // Create timestamp for midnight UTC
    module.insert("make_date".to_string(), Value::NativeFunction {
        name: "make_date".to_string(),
        arity: 3,
        func: |args| {
            match (&args[0], &args[1], &args[2]) {
                (Value::Int(year), Value::Int(month), Value::Int(day)) => {
                    match Utc.with_ymd_and_hms(*year as i32, *month as u32, *day as u32, 0, 0, 0) {
                        chrono::LocalResult::Single(dt) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Ok".to_string(),
                            values: vec![Value::Int(dt.timestamp())],
                        }),
                        _ => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String("Invalid date components".to_string())],
                        }),
                    }
                }
                _ => Err(IntentError::TypeError("make_date() requires 3 integers: (year, month, day)".to_string())),
            }
        },
    });
    
    // ========== Date Arithmetic ==========
    
    // add_seconds(timestamp, seconds) -> Int
    module.insert("add_seconds".to_string(), Value::NativeFunction {
        name: "add_seconds".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::Int(ts), Value::Int(secs)) => {
                    Ok(Value::Int(ts + secs))
                }
                _ => Err(IntentError::TypeError("add_seconds() requires (timestamp: Int, seconds: Int)".to_string())),
            }
        },
    });
    
    // add_minutes(timestamp, minutes) -> Int
    module.insert("add_minutes".to_string(), Value::NativeFunction {
        name: "add_minutes".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::Int(ts), Value::Int(mins)) => {
                    Ok(Value::Int(ts + mins * 60))
                }
                _ => Err(IntentError::TypeError("add_minutes() requires (timestamp: Int, minutes: Int)".to_string())),
            }
        },
    });
    
    // add_hours(timestamp, hours) -> Int
    module.insert("add_hours".to_string(), Value::NativeFunction {
        name: "add_hours".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::Int(ts), Value::Int(hours)) => {
                    Ok(Value::Int(ts + hours * 3600))
                }
                _ => Err(IntentError::TypeError("add_hours() requires (timestamp: Int, hours: Int)".to_string())),
            }
        },
    });
    
    // add_days(timestamp, days) -> Int
    module.insert("add_days".to_string(), Value::NativeFunction {
        name: "add_days".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::Int(ts), Value::Int(days)) => {
                    Ok(Value::Int(ts + days * 86400))
                }
                _ => Err(IntentError::TypeError("add_days() requires (timestamp: Int, days: Int)".to_string())),
            }
        },
    });
    
    // add_weeks(timestamp, weeks) -> Int
    module.insert("add_weeks".to_string(), Value::NativeFunction {
        name: "add_weeks".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::Int(ts), Value::Int(weeks)) => {
                    Ok(Value::Int(ts + weeks * 604800))
                }
                _ => Err(IntentError::TypeError("add_weeks() requires (timestamp: Int, weeks: Int)".to_string())),
            }
        },
    });
    
    // add_months(timestamp, months) -> Int - Calendar-aware month addition
    module.insert("add_months".to_string(), Value::NativeFunction {
        name: "add_months".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::Int(ts), Value::Int(months)) => {
                    let dt = DateTime::from_timestamp(*ts, 0)
                        .ok_or_else(|| IntentError::RuntimeError("Invalid timestamp".to_string()))?;
                    
                    // Proper calendar-aware month addition
                    let year = dt.year();
                    let month = dt.month() as i32;
                    let day = dt.day();
                    
                    let total_months = year * 12 + month - 1 + (*months as i32);
                    let new_year = total_months / 12;
                    let new_month = (total_months % 12 + 1) as u32;
                    
                    // Handle day overflow (e.g., Jan 31 + 1 month = Feb 28/29)
                    let days_in_month = match new_month {
                        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
                        4 | 6 | 9 | 11 => 30,
                        2 => {
                            let is_leap = (new_year % 4 == 0 && new_year % 100 != 0) || (new_year % 400 == 0);
                            if is_leap { 29 } else { 28 }
                        }
                        _ => 30,
                    };
                    let new_day = day.min(days_in_month);
                    
                    match Utc.with_ymd_and_hms(new_year, new_month, new_day, dt.hour(), dt.minute(), dt.second()) {
                        chrono::LocalResult::Single(new_dt) => Ok(Value::Int(new_dt.timestamp())),
                        _ => Err(IntentError::RuntimeError("Invalid date after month addition".to_string())),
                    }
                }
                _ => Err(IntentError::TypeError("add_months() requires (timestamp: Int, months: Int)".to_string())),
            }
        },
    });
    
    // add_years(timestamp, years) -> Int - Calendar-aware year addition
    module.insert("add_years".to_string(), Value::NativeFunction {
        name: "add_years".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::Int(ts), Value::Int(years)) => {
                    let dt = DateTime::from_timestamp(*ts, 0)
                        .ok_or_else(|| IntentError::RuntimeError("Invalid timestamp".to_string()))?;
                    
                    let new_year = dt.year() + (*years as i32);
                    let month = dt.month();
                    let day = dt.day();
                    
                    // Handle Feb 29 on non-leap years
                    let new_day = if month == 2 && day == 29 {
                        let is_leap = (new_year % 4 == 0 && new_year % 100 != 0) || (new_year % 400 == 0);
                        if is_leap { 29 } else { 28 }
                    } else {
                        day
                    };
                    
                    match Utc.with_ymd_and_hms(new_year, month, new_day, dt.hour(), dt.minute(), dt.second()) {
                        chrono::LocalResult::Single(new_dt) => Ok(Value::Int(new_dt.timestamp())),
                        _ => Err(IntentError::RuntimeError("Invalid date after year addition".to_string())),
                    }
                }
                _ => Err(IntentError::TypeError("add_years() requires (timestamp: Int, years: Int)".to_string())),
            }
        },
    });
    
    // diff(timestamp1, timestamp2) -> Map - Difference between two timestamps
    // Returns { seconds, minutes, hours, days }
    module.insert("diff".to_string(), Value::NativeFunction {
        name: "diff".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::Int(ts1), Value::Int(ts2)) => {
                    let diff_secs = ts1 - ts2;
                    let mut map = HashMap::new();
                    map.insert("seconds".to_string(), Value::Int(diff_secs));
                    map.insert("minutes".to_string(), Value::Int(diff_secs / 60));
                    map.insert("hours".to_string(), Value::Int(diff_secs / 3600));
                    map.insert("days".to_string(), Value::Int(diff_secs / 86400));
                    Ok(Value::Map(map))
                }
                _ => Err(IntentError::TypeError("diff() requires two timestamps".to_string())),
            }
        },
    });
    
    // ========== Comparisons ==========
    
    // before(timestamp1, timestamp2) -> Bool - Is ts1 before ts2?
    module.insert("before".to_string(), Value::NativeFunction {
        name: "before".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::Int(ts1), Value::Int(ts2)) => Ok(Value::Bool(ts1 < ts2)),
                _ => Err(IntentError::TypeError("before() requires two timestamps".to_string())),
            }
        },
    });
    
    // after(timestamp1, timestamp2) -> Bool - Is ts1 after ts2?
    module.insert("after".to_string(), Value::NativeFunction {
        name: "after".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::Int(ts1), Value::Int(ts2)) => Ok(Value::Bool(ts1 > ts2)),
                _ => Err(IntentError::TypeError("after() requires two timestamps".to_string())),
            }
        },
    });
    
    // equal(timestamp1, timestamp2) -> Bool - Are timestamps equal?
    module.insert("equal".to_string(), Value::NativeFunction {
        name: "equal".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::Int(ts1), Value::Int(ts2)) => Ok(Value::Bool(ts1 == ts2)),
                _ => Err(IntentError::TypeError("equal() requires two timestamps".to_string())),
            }
        },
    });
    
    // ========== Date Components ==========
    
    // year(timestamp) -> Int
    module.insert("year".to_string(), Value::NativeFunction {
        name: "year".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::Int(ts) => {
                    let dt = DateTime::from_timestamp(*ts, 0)
                        .ok_or_else(|| IntentError::RuntimeError("Invalid timestamp".to_string()))?;
                    Ok(Value::Int(dt.year() as i64))
                }
                _ => Err(IntentError::TypeError("year() requires a timestamp".to_string())),
            }
        },
    });
    
    // month(timestamp) -> Int (1-12)
    module.insert("month".to_string(), Value::NativeFunction {
        name: "month".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::Int(ts) => {
                    let dt = DateTime::from_timestamp(*ts, 0)
                        .ok_or_else(|| IntentError::RuntimeError("Invalid timestamp".to_string()))?;
                    Ok(Value::Int(dt.month() as i64))
                }
                _ => Err(IntentError::TypeError("month() requires a timestamp".to_string())),
            }
        },
    });
    
    // day(timestamp) -> Int (1-31)
    module.insert("day".to_string(), Value::NativeFunction {
        name: "day".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::Int(ts) => {
                    let dt = DateTime::from_timestamp(*ts, 0)
                        .ok_or_else(|| IntentError::RuntimeError("Invalid timestamp".to_string()))?;
                    Ok(Value::Int(dt.day() as i64))
                }
                _ => Err(IntentError::TypeError("day() requires a timestamp".to_string())),
            }
        },
    });
    
    // hour(timestamp) -> Int (0-23)
    module.insert("hour".to_string(), Value::NativeFunction {
        name: "hour".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::Int(ts) => {
                    let dt = DateTime::from_timestamp(*ts, 0)
                        .ok_or_else(|| IntentError::RuntimeError("Invalid timestamp".to_string()))?;
                    Ok(Value::Int(dt.hour() as i64))
                }
                _ => Err(IntentError::TypeError("hour() requires a timestamp".to_string())),
            }
        },
    });
    
    // minute(timestamp) -> Int (0-59)
    module.insert("minute".to_string(), Value::NativeFunction {
        name: "minute".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::Int(ts) => {
                    let dt = DateTime::from_timestamp(*ts, 0)
                        .ok_or_else(|| IntentError::RuntimeError("Invalid timestamp".to_string()))?;
                    Ok(Value::Int(dt.minute() as i64))
                }
                _ => Err(IntentError::TypeError("minute() requires a timestamp".to_string())),
            }
        },
    });
    
    // second(timestamp) -> Int (0-59)
    module.insert("second".to_string(), Value::NativeFunction {
        name: "second".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::Int(ts) => {
                    let dt = DateTime::from_timestamp(*ts, 0)
                        .ok_or_else(|| IntentError::RuntimeError("Invalid timestamp".to_string()))?;
                    Ok(Value::Int(dt.second() as i64))
                }
                _ => Err(IntentError::TypeError("second() requires a timestamp".to_string())),
            }
        },
    });
    
    // weekday(timestamp) -> Int (0=Sunday, 1=Monday, ..., 6=Saturday)
    module.insert("weekday".to_string(), Value::NativeFunction {
        name: "weekday".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::Int(ts) => {
                    let dt = DateTime::from_timestamp(*ts, 0)
                        .ok_or_else(|| IntentError::RuntimeError("Invalid timestamp".to_string()))?;
                    Ok(Value::Int(dt.weekday().num_days_from_sunday() as i64))
                }
                _ => Err(IntentError::TypeError("weekday() requires a timestamp".to_string())),
            }
        },
    });
    
    // weekday_name(timestamp) -> String ("Sunday", "Monday", etc.)
    module.insert("weekday_name".to_string(), Value::NativeFunction {
        name: "weekday_name".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::Int(ts) => {
                    let dt = DateTime::from_timestamp(*ts, 0)
                        .ok_or_else(|| IntentError::RuntimeError("Invalid timestamp".to_string()))?;
                    let name = match dt.weekday() {
                        chrono::Weekday::Sun => "Sunday",
                        chrono::Weekday::Mon => "Monday",
                        chrono::Weekday::Tue => "Tuesday",
                        chrono::Weekday::Wed => "Wednesday",
                        chrono::Weekday::Thu => "Thursday",
                        chrono::Weekday::Fri => "Friday",
                        chrono::Weekday::Sat => "Saturday",
                    };
                    Ok(Value::String(name.to_string()))
                }
                _ => Err(IntentError::TypeError("weekday_name() requires a timestamp".to_string())),
            }
        },
    });
    
    // month_name(timestamp) -> String ("January", "February", etc.)
    module.insert("month_name".to_string(), Value::NativeFunction {
        name: "month_name".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::Int(ts) => {
                    let dt = DateTime::from_timestamp(*ts, 0)
                        .ok_or_else(|| IntentError::RuntimeError("Invalid timestamp".to_string()))?;
                    let name = match dt.month() {
                        1 => "January", 2 => "February", 3 => "March",
                        4 => "April", 5 => "May", 6 => "June",
                        7 => "July", 8 => "August", 9 => "September",
                        10 => "October", 11 => "November", 12 => "December",
                        _ => "Unknown",
                    };
                    Ok(Value::String(name.to_string()))
                }
                _ => Err(IntentError::TypeError("month_name() requires a timestamp".to_string())),
            }
        },
    });
    
    // day_of_year(timestamp) -> Int (1-366)
    module.insert("day_of_year".to_string(), Value::NativeFunction {
        name: "day_of_year".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::Int(ts) => {
                    let dt = DateTime::from_timestamp(*ts, 0)
                        .ok_or_else(|| IntentError::RuntimeError("Invalid timestamp".to_string()))?;
                    Ok(Value::Int(dt.ordinal() as i64))
                }
                _ => Err(IntentError::TypeError("day_of_year() requires a timestamp".to_string())),
            }
        },
    });
    
    // is_leap_year(timestamp) -> Bool
    module.insert("is_leap_year".to_string(), Value::NativeFunction {
        name: "is_leap_year".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::Int(ts) => {
                    let dt = DateTime::from_timestamp(*ts, 0)
                        .ok_or_else(|| IntentError::RuntimeError("Invalid timestamp".to_string()))?;
                    let year = dt.year();
                    let is_leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
                    Ok(Value::Bool(is_leap))
                }
                _ => Err(IntentError::TypeError("is_leap_year() requires a timestamp".to_string())),
            }
        },
    });
    
    // ========== Utilities ==========
    
    // sleep(millis) -> Unit - Sleep for milliseconds
    module.insert("sleep".to_string(), Value::NativeFunction {
        name: "sleep".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::Int(ms) => {
                    if *ms < 0 {
                        return Err(IntentError::RuntimeError("sleep() requires non-negative milliseconds".to_string()));
                    }
                    std::thread::sleep(Duration::from_millis(*ms as u64));
                    Ok(Value::Unit)
                }
                _ => Err(IntentError::TypeError("sleep() requires an integer (milliseconds)".to_string())),
            }
        },
    });
    
    // elapsed(start_millis) -> Int - Milliseconds since start
    module.insert("elapsed".to_string(), Value::NativeFunction {
        name: "elapsed".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::Int(start) => {
                    let now = Utc::now().timestamp_millis();
                    Ok(Value::Int(now - start))
                }
                _ => Err(IntentError::TypeError("elapsed() requires a start timestamp".to_string())),
            }
        },
    });
    
    // ========== Timezone Utilities ==========
    
    // list_timezones() -> [String] - List common available timezones
    module.insert("list_timezones".to_string(), Value::NativeFunction {
        name: "list_timezones".to_string(),
        arity: 0,
        func: |_args| {
            let common = vec![
                "UTC", "America/New_York", "America/Chicago", "America/Denver", 
                "America/Los_Angeles", "America/Phoenix", "America/Anchorage",
                "Pacific/Honolulu", "Europe/London", "Europe/Paris", "Europe/Berlin",
                "Europe/Moscow", "Asia/Tokyo", "Asia/Shanghai", "Asia/Singapore",
                "Asia/Dubai", "Asia/Kolkata", "Australia/Sydney", "Australia/Melbourne",
                "Pacific/Auckland", "America/Toronto", "America/Vancouver",
                "America/Mexico_City", "America/Sao_Paulo", "Africa/Cairo",
                "Africa/Johannesburg",
            ];
            Ok(Value::Array(common.iter().map(|s| Value::String(s.to_string())).collect()))
        },
    });
    
    // ========== Constants ==========
    
    // Duration constants in seconds
    module.insert("SECOND".to_string(), Value::Int(1));
    module.insert("MINUTE".to_string(), Value::Int(60));
    module.insert("HOUR".to_string(), Value::Int(3600));
    module.insert("DAY".to_string(), Value::Int(86400));
    module.insert("WEEK".to_string(), Value::Int(604800));
    
    // ========== Backward Compatibility ==========
    
    // format_timestamp - alias for format
    module.insert("format_timestamp".to_string(), Value::NativeFunction {
        name: "format_timestamp".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::Int(ts), Value::String(fmt)) => {
                    let dt = DateTime::from_timestamp(*ts, 0)
                        .ok_or_else(|| IntentError::RuntimeError("Invalid timestamp".to_string()))?;
                    Ok(Value::String(dt.format(fmt).to_string()))
                }
                _ => Err(IntentError::TypeError("format_timestamp() requires int and format string".to_string())),
            }
        },
    });
    
    // duration_secs - kept for compatibility
    module.insert("duration_secs".to_string(), Value::NativeFunction {
        name: "duration_secs".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::Int(secs) => {
                    let mut map = HashMap::new();
                    map.insert("secs".to_string(), Value::Int(*secs));
                    map.insert("millis".to_string(), Value::Int(*secs * 1000));
                    map.insert("nanos".to_string(), Value::Int(*secs * 1_000_000_000));
                    Ok(Value::Map(map))
                }
                _ => Err(IntentError::TypeError("duration_secs() requires an integer".to_string())),
            }
        },
    });
    
    // duration_millis - kept for compatibility
    module.insert("duration_millis".to_string(), Value::NativeFunction {
        name: "duration_millis".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::Int(ms) => {
                    let mut map = HashMap::new();
                    map.insert("secs".to_string(), Value::Int(*ms / 1000));
                    map.insert("millis".to_string(), Value::Int(*ms));
                    map.insert("nanos".to_string(), Value::Int(*ms * 1_000_000));
                    Ok(Value::Map(map))
                }
                _ => Err(IntentError::TypeError("duration_millis() requires an integer".to_string())),
            }
        },
    });
    
    module
}
