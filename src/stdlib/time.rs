//! std/time module - Comprehensive time and date operations
//!
//! Provides Go-like time handling with timezone support, date parsing,
//! arithmetic, and formatting.

use crate::error::IntentError;
use crate::interpreter::Value;
use chrono::{DateTime, Datelike, NaiveDateTime, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use std::collections::HashMap;
use std::time::Duration;

/// Helper to create a datetime map from chrono DateTime
fn datetime_to_map<T: TimeZone>(dt: &DateTime<T>, tz_name: &str) -> HashMap<String, Value>
where
    T::Offset: std::fmt::Display,
{
    let mut map = HashMap::new();
    map.insert("year".to_string(), Value::Int(dt.year() as i64));
    map.insert("month".to_string(), Value::Int(dt.month() as i64));
    map.insert("day".to_string(), Value::Int(dt.day() as i64));
    map.insert("hour".to_string(), Value::Int(dt.hour() as i64));
    map.insert("minute".to_string(), Value::Int(dt.minute() as i64));
    map.insert("second".to_string(), Value::Int(dt.second() as i64));
    map.insert("nanosecond".to_string(), Value::Int(dt.nanosecond() as i64));
    map.insert(
        "weekday".to_string(),
        Value::Int(dt.weekday().num_days_from_sunday() as i64),
    );
    map.insert("day_of_year".to_string(), Value::Int(dt.ordinal() as i64));
    map.insert("timestamp".to_string(), Value::Int(dt.timestamp()));
    map.insert("timezone".to_string(), Value::String(tz_name.to_string()));
    map.insert("offset".to_string(), Value::String(dt.offset().to_string()));
    map
}

/// Parse timezone string to chrono_tz::Tz
fn parse_timezone(tz: &str) -> Result<Tz, IntentError> {
    tz.parse::<Tz>().map_err(|_| {
        IntentError::runtime_error(format!(
            "Invalid timezone: '{}'. Use IANA format like 'America/New_York'",
            tz
        ))
    })
}

/// Initialize the std/time module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();

    // ========== Current Time ==========

    // @ntnt now
    // @module std/time
    // @module_description Date, time, and duration operations
    // @signature now() -> Int
    // Returns the current Unix timestamp in seconds (UTC).
    //
    // Provides the current wall-clock time as a Unix epoch timestamp.
    // Use now_millis() or now_nanos() for higher precision.
    // @returns The current Unix timestamp in seconds
    // @see_also now_millis, now_nanos, elapsed
    // @since v0.1.0
    // @tags #time
    // @example now() => 1700000000 ~ "Returns current Unix timestamp (value varies)"
    module.insert(
        "now".to_string(),
        Value::NativeFunction {
            name: "now".to_string(),
            arity: 0,
            max_arity: 0,
            requires: None,
            func: |_args| Ok(Value::Int(Utc::now().timestamp())),
        },
    );

    // @ntnt now_millis
    // @module std/time
    // @signature now_millis() -> Int
    // Returns the current Unix timestamp in milliseconds.
    //
    // Higher precision variant of now(). Useful for measuring elapsed
    // time or generating unique-ish identifiers.
    // @returns The current Unix timestamp in milliseconds
    // @see_also now, now_nanos, elapsed
    // @since v0.1.0
    // @tags #time
    // @example now_millis() => 1700000000000 ~ "Returns current timestamp in ms (value varies)"
    module.insert(
        "now_millis".to_string(),
        Value::NativeFunction {
            name: "now_millis".to_string(),
            arity: 0,
            max_arity: 0,
            requires: None,
            func: |_args| Ok(Value::Int(Utc::now().timestamp_millis())),
        },
    );

    // @ntnt now_nanos
    // @module std/time
    // @signature now_nanos() -> Int
    // Returns the current Unix timestamp in nanoseconds.
    //
    // Highest precision variant of now(). May fail if the timestamp
    // is out of range for nanosecond representation.
    // @returns The current Unix timestamp in nanoseconds
    // @see_also now, now_millis
    // @since v0.1.0
    // @tags #time
    // @example now_nanos() => 1700000000000000000 ~ "Returns current timestamp in ns (value varies)"
    // @error RuntimeError ~ "Timestamp out of range for nanoseconds" fix: "Use now() or now_millis() for timestamps outside nanosecond range"
    module.insert(
        "now_nanos".to_string(),
        Value::NativeFunction {
            name: "now_nanos".to_string(),
            arity: 0,
            max_arity: 0,
            requires: None,
            func: |_args| match Utc::now().timestamp_nanos_opt() {
                Some(nanos) => Ok(Value::Int(nanos)),
                None => Err(IntentError::runtime_error(
                    "Timestamp out of range for nanoseconds".to_string(),
                )),
            },
        },
    );

    // ========== Timezone Conversion ==========

    // @ntnt to_timezone
    // @module std/time
    // @signature to_timezone(timestamp: Int, timezone: String) -> Map
    // Converts a Unix timestamp to a datetime map in the specified timezone.
    //
    // Returns a map with keys: year, month, day, hour, minute, second,
    // nanosecond, weekday, day_of_year, timestamp, timezone, offset.
    // @param timestamp Unix timestamp in seconds
    // @param timezone IANA timezone string (e.g., "America/New_York")
    // @returns Map with datetime components in the given timezone
    // @see_also to_utc, format_in, list_timezones
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example to_timezone(0, "UTC").year => 1970 ~ "Epoch in UTC is 1970"
    // @error TypeError ~ "to_timezone() requires (timestamp: Int, timezone: String)" fix: "Pass an Int timestamp and a String timezone"
    // @error RuntimeError ~ "Invalid timestamp" fix: "Ensure the timestamp is a valid Unix epoch value"
    // @error RuntimeError ~ "Invalid timezone" fix: "Use IANA format like 'America/New_York'"
    module.insert(
        "to_timezone".to_string(),
        Value::NativeFunction {
            name: "to_timezone".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| match (&args[0], &args[1]) {
                (Value::Int(ts), Value::String(tz_str)) => {
                    let tz = parse_timezone(tz_str)?;
                    let dt = DateTime::from_timestamp(*ts, 0)
                        .ok_or_else(|| IntentError::runtime_error("Invalid timestamp".to_string()))?
                        .with_timezone(&tz);
                    Ok(Value::Map(datetime_to_map(&dt, tz_str)))
                }
                _ => Err(IntentError::type_error(
                    "to_timezone() requires (timestamp: Int, timezone: String)".to_string(),
                )),
            },
        },
    );

    // @ntnt to_utc
    // @module std/time
    // @signature to_utc(timestamp: Int) -> Map
    // Converts a Unix timestamp to a UTC datetime map.
    //
    // Returns a map with keys: year, month, day, hour, minute, second,
    // nanosecond, weekday, day_of_year, timestamp, timezone, offset.
    // @param timestamp Unix timestamp in seconds
    // @returns Map with datetime components in UTC
    // @see_also to_timezone, format, year, month, day
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example to_utc(0).year => 1970 ~ "Epoch is January 1, 1970"
    // @error TypeError ~ "to_utc() requires a timestamp" fix: "Pass an Int timestamp"
    // @error RuntimeError ~ "Invalid timestamp" fix: "Ensure the timestamp is a valid Unix epoch value"
    module.insert(
        "to_utc".to_string(),
        Value::NativeFunction {
            name: "to_utc".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::Int(ts) => {
                    let dt = DateTime::from_timestamp(*ts, 0).ok_or_else(|| {
                        IntentError::runtime_error("Invalid timestamp".to_string())
                    })?;
                    Ok(Value::Map(datetime_to_map(&dt, "UTC")))
                }
                _ => Err(IntentError::type_error(
                    "to_utc() requires a timestamp".to_string(),
                )),
            },
        },
    );

    // ========== Formatting ==========

    // @ntnt format
    // @module std/time
    // @signature format(timestamp: Int, format_str: String) -> String
    // Formats a Unix timestamp as a string using a strftime format pattern (UTC).
    //
    // Supports standard strftime directives: %Y %m %d %H %M %S %f %Z %z %a %A %b %B %j %U %W %w.
    // To format in a specific timezone, use format_in() instead.
    // @param timestamp Unix timestamp in seconds
    // @param format_str strftime-compatible format string
    // @returns Formatted date/time string
    // @see_also format_in, to_iso, format_timestamp
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example format(0, "%Y-%m-%d") => "1970-01-01" ~ "Epoch date formatted"
    // @error TypeError ~ "format() requires (timestamp: Int, format: String)" fix: "Pass an Int timestamp and a String format"
    // @error RuntimeError ~ "Invalid timestamp" fix: "Ensure the timestamp is a valid Unix epoch value"
    module.insert(
        "format".to_string(),
        Value::NativeFunction {
            name: "format".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| match (&args[0], &args[1]) {
                (Value::Int(ts), Value::String(fmt)) => {
                    let dt = DateTime::from_timestamp(*ts, 0).ok_or_else(|| {
                        IntentError::runtime_error("Invalid timestamp".to_string())
                    })?;
                    Ok(Value::String(dt.format(fmt).to_string()))
                }
                _ => Err(IntentError::type_error(
                    "format() requires (timestamp: Int, format: String)".to_string(),
                )),
            },
        },
    );

    // @ntnt format_in
    // @module std/time
    // @signature format_in(timestamp: Int, timezone: String, format_str: String) -> String
    // Formats a Unix timestamp as a string in the specified timezone.
    //
    // Combines timezone conversion and formatting in a single call.
    // Uses strftime directives: %Y %m %d %H %M %S %f %Z %z %a %A %b %B %j %U %W %w.
    // @param timestamp Unix timestamp in seconds
    // @param timezone IANA timezone string (e.g., "America/New_York")
    // @param format_str strftime-compatible format string
    // @returns Formatted date/time string in the given timezone
    // @see_also format, to_timezone, to_iso, list_timezones
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example format_in(0, "UTC", "%Y-%m-%d %H:%M:%S") => "1970-01-01 00:00:00" ~ "Epoch in UTC"
    // @error TypeError ~ "format_in() requires (timestamp: Int, timezone: String, format: String)" fix: "Pass Int, String, String arguments"
    // @error RuntimeError ~ "Invalid timestamp" fix: "Ensure the timestamp is a valid Unix epoch value"
    // @error RuntimeError ~ "Invalid timezone" fix: "Use IANA format like 'America/New_York'"
    module.insert(
        "format_in".to_string(),
        Value::NativeFunction {
            name: "format_in".to_string(),
            arity: 3,
            max_arity: 3,
            requires: None,
            func: |args| match (&args[0], &args[1], &args[2]) {
                (Value::Int(ts), Value::String(tz_str), Value::String(fmt)) => {
                    let tz = parse_timezone(tz_str)?;
                    let dt = DateTime::from_timestamp(*ts, 0)
                        .ok_or_else(|| IntentError::runtime_error("Invalid timestamp".to_string()))?
                        .with_timezone(&tz);
                    Ok(Value::String(dt.format(fmt).to_string()))
                }
                _ => Err(IntentError::type_error(
                    "format_in() requires (timestamp: Int, timezone: String, format: String)"
                        .to_string(),
                )),
            },
        },
    );

    // @ntnt to_iso
    // @module std/time
    // @signature to_iso(timestamp: Int) -> String
    // Formats a Unix timestamp as an ISO 8601 (RFC 3339) string.
    //
    // Produces a standard ISO 8601 datetime string in UTC, suitable for
    // APIs, JSON serialization, and interoperability.
    // @param timestamp Unix timestamp in seconds
    // @returns ISO 8601 formatted string (e.g., "1970-01-01T00:00:00+00:00")
    // @see_also parse_iso, format, format_in
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example to_iso(0) => "1970-01-01T00:00:00+00:00" ~ "Epoch as ISO 8601"
    // @error TypeError ~ "to_iso() requires a timestamp" fix: "Pass an Int timestamp"
    // @error RuntimeError ~ "Invalid timestamp" fix: "Ensure the timestamp is a valid Unix epoch value"
    module.insert(
        "to_iso".to_string(),
        Value::NativeFunction {
            name: "to_iso".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::Int(ts) => {
                    let dt = DateTime::from_timestamp(*ts, 0).ok_or_else(|| {
                        IntentError::runtime_error("Invalid timestamp".to_string())
                    })?;
                    Ok(Value::String(dt.to_rfc3339()))
                }
                _ => Err(IntentError::type_error(
                    "to_iso() requires a timestamp".to_string(),
                )),
            },
        },
    );

    // ========== Parsing ==========

    // @ntnt parse_datetime
    // @module std/time
    // @signature parse_datetime(date_str: String, format_str: String) -> Result<Int, String>
    // Parses a date/time string into a Unix timestamp using the given format.
    //
    // Uses strftime format directives to parse the input string. Returns a Result
    // so parsing failures are handled gracefully without raising exceptions.
    // The parsed datetime is treated as UTC.
    // @param date_str The date/time string to parse
    // @param format_str strftime-compatible format string
    // @returns Result containing the Unix timestamp on success, or an error message on failure
    // @see_also parse_iso, format, make_time
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example parse_datetime("2024-01-01 00:00:00", "%Y-%m-%d %H:%M:%S") => Result::Ok(1704067200) ~ "Parsed to timestamp"
    // @error TypeError ~ "parse_datetime() requires (date_str: String, format: String)" fix: "Pass two String arguments"
    module.insert(
        "parse_datetime".to_string(),
        Value::NativeFunction {
            name: "parse_datetime".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(date_str), Value::String(fmt)) => {
                    match NaiveDateTime::parse_from_str(date_str, fmt) {
                        Ok(naive) => {
                            let dt = naive.and_utc();
                            Ok(Value::ok(Value::Int(dt.timestamp())))
                        }
                        Err(e) => Ok(Value::err(Value::String(format!("Parse error: {}", e)))),
                    }
                }
                _ => Err(IntentError::type_error(
                    "parse_datetime() requires (date_str: String, format: String)".to_string(),
                )),
            },
        },
    );

    // @ntnt parse_iso
    // @module std/time
    // @signature parse_iso(iso_str: String) -> Result<Int, String>
    // Parses an ISO 8601 (RFC 3339) string into a Unix timestamp.
    //
    // Accepts standard ISO 8601 datetime strings with timezone offset.
    // Returns a Result so parsing failures are handled gracefully.
    // @param iso_str An ISO 8601 / RFC 3339 formatted string
    // @returns Result containing the Unix timestamp on success, or an error message on failure
    // @see_also to_iso, parse_datetime
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example parse_iso("1970-01-01T00:00:00+00:00") => Result::Ok(0) ~ "Epoch parsed from ISO"
    // @error TypeError ~ "parse_iso() requires a string" fix: "Pass a String argument"
    module.insert(
        "parse_iso".to_string(),
        Value::NativeFunction {
            name: "parse_iso".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(s) => match DateTime::parse_from_rfc3339(s) {
                    Ok(dt) => Ok(Value::ok(Value::Int(dt.timestamp()))),
                    Err(e) => Ok(Value::err(Value::String(format!("Parse error: {}", e)))),
                },
                _ => Err(IntentError::type_error(
                    "parse_iso() requires a string".to_string(),
                )),
            },
        },
    );

    // ========== Date Creation ==========

    // @ntnt make_time
    // @module std/time
    // @signature make_time(year: Int, month: Int, day: Int, hour: Int, minute: Int, second: Int) -> Result<Int, String>
    // Creates a Unix timestamp from individual date and time components (UTC).
    //
    // Validates the components and returns a Result. Invalid combinations
    // (e.g., month 13 or day 32) produce an Err variant.
    // @param year The year (e.g., 2024)
    // @param month The month (1-12)
    // @param day The day (1-31)
    // @param hour The hour (0-23)
    // @param minute The minute (0-59)
    // @param second The second (0-59)
    // @returns Result containing the Unix timestamp on success, or an error message on failure
    // @see_also make_date, parse_datetime, to_utc
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example make_time(1970, 1, 1, 0, 0, 0) => Result::Ok(0) ~ "Epoch from components"
    // @error TypeError ~ "make_time() requires 6 integers: (year, month, day, hour, minute, second)" fix: "Pass six Int arguments"
    module.insert(
        "make_time".to_string(),
        Value::NativeFunction {
            name: "make_time".to_string(),
            arity: 6,
            max_arity: 6,
            requires: None,
            func: |args| match (&args[0], &args[1], &args[2], &args[3], &args[4], &args[5]) {
                (
                    Value::Int(year),
                    Value::Int(month),
                    Value::Int(day),
                    Value::Int(hour),
                    Value::Int(minute),
                    Value::Int(second),
                ) => {
                    match Utc.with_ymd_and_hms(
                        *year as i32,
                        *month as u32,
                        *day as u32,
                        *hour as u32,
                        *minute as u32,
                        *second as u32,
                    ) {
                        chrono::LocalResult::Single(dt) => {
                            Ok(Value::ok(Value::Int(dt.timestamp())))
                        }
                        _ => Ok(Value::err(Value::String(
                            "Invalid date/time components".to_string(),
                        ))),
                    }
                }
                _ => Err(IntentError::type_error(
                    "make_time() requires 6 integers: (year, month, day, hour, minute, second)"
                        .to_string(),
                )),
            },
        },
    );

    // @ntnt make_date
    // @module std/time
    // @signature make_date(year: Int, month: Int, day: Int) -> Result<Int, String>
    // Creates a Unix timestamp for midnight UTC from date components.
    //
    // Shorthand for make_time(year, month, day, 0, 0, 0). Returns a Result
    // so invalid dates are handled gracefully.
    // @param year The year (e.g., 2024)
    // @param month The month (1-12)
    // @param day The day (1-31)
    // @returns Result containing the Unix timestamp at midnight UTC, or an error message on failure
    // @see_also make_time, parse_datetime, year, month, day
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example make_date(1970, 1, 1) => Result::Ok(0) ~ "Epoch date"
    // @error TypeError ~ "make_date() requires 3 integers: (year, month, day)" fix: "Pass three Int arguments"
    module.insert(
        "make_date".to_string(),
        Value::NativeFunction {
            name: "make_date".to_string(),
            arity: 3,
            max_arity: 3,
            requires: None,
            func: |args| match (&args[0], &args[1], &args[2]) {
                (Value::Int(year), Value::Int(month), Value::Int(day)) => {
                    match Utc.with_ymd_and_hms(*year as i32, *month as u32, *day as u32, 0, 0, 0) {
                        chrono::LocalResult::Single(dt) => {
                            Ok(Value::ok(Value::Int(dt.timestamp())))
                        }
                        _ => Ok(Value::err(Value::String(
                            "Invalid date components".to_string(),
                        ))),
                    }
                }
                _ => Err(IntentError::type_error(
                    "make_date() requires 3 integers: (year, month, day)".to_string(),
                )),
            },
        },
    );

    // ========== Date Arithmetic ==========

    // @ntnt add_seconds
    // @module std/time
    // @signature add_seconds(timestamp: Int, seconds: Int) -> Int
    // Adds seconds to a Unix timestamp.
    //
    // Simple arithmetic addition. Use negative values to subtract.
    // @param timestamp Unix timestamp in seconds
    // @param seconds Number of seconds to add (negative to subtract)
    // @returns New Unix timestamp with seconds added
    // @see_also add_minutes, add_hours, add_days, add_weeks, diff
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example add_seconds(0, 60) => 60 ~ "Add 60 seconds to epoch"
    // @error TypeError ~ "add_seconds() requires (timestamp: Int, seconds: Int)" fix: "Pass two Int arguments"
    module.insert(
        "add_seconds".to_string(),
        Value::NativeFunction {
            name: "add_seconds".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| match (&args[0], &args[1]) {
                (Value::Int(ts), Value::Int(secs)) => Ok(Value::Int(ts + secs)),
                _ => Err(IntentError::type_error(
                    "add_seconds() requires (timestamp: Int, seconds: Int)".to_string(),
                )),
            },
        },
    );

    // @ntnt add_minutes
    // @module std/time
    // @signature add_minutes(timestamp: Int, minutes: Int) -> Int
    // Adds minutes to a Unix timestamp.
    //
    // Multiplies minutes by 60 and adds to the timestamp. Use negative values to subtract.
    // @param timestamp Unix timestamp in seconds
    // @param minutes Number of minutes to add (negative to subtract)
    // @returns New Unix timestamp with minutes added
    // @see_also add_seconds, add_hours, add_days, add_weeks, diff
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example add_minutes(0, 5) => 300 ~ "Add 5 minutes to epoch"
    // @error TypeError ~ "add_minutes() requires (timestamp: Int, minutes: Int)" fix: "Pass two Int arguments"
    module.insert(
        "add_minutes".to_string(),
        Value::NativeFunction {
            name: "add_minutes".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| match (&args[0], &args[1]) {
                (Value::Int(ts), Value::Int(mins)) => Ok(Value::Int(ts + mins * 60)),
                _ => Err(IntentError::type_error(
                    "add_minutes() requires (timestamp: Int, minutes: Int)".to_string(),
                )),
            },
        },
    );

    // @ntnt add_hours
    // @module std/time
    // @signature add_hours(timestamp: Int, hours: Int) -> Int
    // Adds hours to a Unix timestamp.
    //
    // Multiplies hours by 3600 and adds to the timestamp. Use negative values to subtract.
    // @param timestamp Unix timestamp in seconds
    // @param hours Number of hours to add (negative to subtract)
    // @returns New Unix timestamp with hours added
    // @see_also add_seconds, add_minutes, add_days, add_weeks, diff
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example add_hours(0, 2) => 7200 ~ "Add 2 hours to epoch"
    // @error TypeError ~ "add_hours() requires (timestamp: Int, hours: Int)" fix: "Pass two Int arguments"
    module.insert(
        "add_hours".to_string(),
        Value::NativeFunction {
            name: "add_hours".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| match (&args[0], &args[1]) {
                (Value::Int(ts), Value::Int(hours)) => Ok(Value::Int(ts + hours * 3600)),
                _ => Err(IntentError::type_error(
                    "add_hours() requires (timestamp: Int, hours: Int)".to_string(),
                )),
            },
        },
    );

    // @ntnt add_days
    // @module std/time
    // @signature add_days(timestamp: Int, days: Int) -> Int
    // Adds days to a Unix timestamp.
    //
    // Multiplies days by 86400 and adds to the timestamp. Use negative values to subtract.
    // Not calendar-aware (always 86400 seconds per day). For DST-sensitive
    // calculations, use to_timezone() with manual adjustment.
    // @param timestamp Unix timestamp in seconds
    // @param days Number of days to add (negative to subtract)
    // @returns New Unix timestamp with days added
    // @see_also add_seconds, add_hours, add_weeks, add_months, diff
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example add_days(0, 1) => 86400 ~ "Add 1 day to epoch"
    // @error TypeError ~ "add_days() requires (timestamp: Int, days: Int)" fix: "Pass two Int arguments"
    module.insert(
        "add_days".to_string(),
        Value::NativeFunction {
            name: "add_days".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| match (&args[0], &args[1]) {
                (Value::Int(ts), Value::Int(days)) => Ok(Value::Int(ts + days * 86400)),
                _ => Err(IntentError::type_error(
                    "add_days() requires (timestamp: Int, days: Int)".to_string(),
                )),
            },
        },
    );

    // @ntnt add_weeks
    // @module std/time
    // @signature add_weeks(timestamp: Int, weeks: Int) -> Int
    // Adds weeks to a Unix timestamp.
    //
    // Multiplies weeks by 604800 (7 * 86400) and adds to the timestamp.
    // Use negative values to subtract.
    // @param timestamp Unix timestamp in seconds
    // @param weeks Number of weeks to add (negative to subtract)
    // @returns New Unix timestamp with weeks added
    // @see_also add_days, add_months, add_years, diff
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example add_weeks(0, 1) => 604800 ~ "Add 1 week to epoch"
    // @error TypeError ~ "add_weeks() requires (timestamp: Int, weeks: Int)" fix: "Pass two Int arguments"
    module.insert(
        "add_weeks".to_string(),
        Value::NativeFunction {
            name: "add_weeks".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| match (&args[0], &args[1]) {
                (Value::Int(ts), Value::Int(weeks)) => Ok(Value::Int(ts + weeks * 604800)),
                _ => Err(IntentError::type_error(
                    "add_weeks() requires (timestamp: Int, weeks: Int)".to_string(),
                )),
            },
        },
    );

    // @ntnt add_months
    // @module std/time
    // @signature add_months(timestamp: Int, months: Int) -> Int
    // Adds months to a Unix timestamp with calendar-aware logic.
    //
    // Properly handles month boundaries and varying month lengths.
    // For example, Jan 31 + 1 month = Feb 28 (or 29 in a leap year).
    // Preserves the time-of-day component.
    // @param timestamp Unix timestamp in seconds
    // @param months Number of months to add (negative to subtract)
    // @returns New Unix timestamp with months added
    // @see_also add_years, add_days, add_weeks, diff
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example add_months(0, 1) => 2678400 ~ "Add 1 month to epoch (Jan->Feb)"
    // @error TypeError ~ "add_months() requires (timestamp: Int, months: Int)" fix: "Pass two Int arguments"
    // @error RuntimeError ~ "Invalid timestamp" fix: "Ensure the timestamp is a valid Unix epoch value"
    // @error RuntimeError ~ "Invalid date after month addition" fix: "The resulting date is out of representable range"
    module.insert(
        "add_months".to_string(),
        Value::NativeFunction {
            name: "add_months".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                match (&args[0], &args[1]) {
                    (Value::Int(ts), Value::Int(months)) => {
                        let dt = DateTime::from_timestamp(*ts, 0).ok_or_else(|| {
                            IntentError::runtime_error("Invalid timestamp".to_string())
                        })?;

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
                                let is_leap = (new_year % 4 == 0 && new_year % 100 != 0)
                                    || (new_year % 400 == 0);
                                if is_leap {
                                    29
                                } else {
                                    28
                                }
                            }
                            _ => 30,
                        };
                        let new_day = day.min(days_in_month);

                        match Utc.with_ymd_and_hms(
                            new_year,
                            new_month,
                            new_day,
                            dt.hour(),
                            dt.minute(),
                            dt.second(),
                        ) {
                            chrono::LocalResult::Single(new_dt) => {
                                Ok(Value::Int(new_dt.timestamp()))
                            }
                            _ => Err(IntentError::runtime_error(
                                "Invalid date after month addition".to_string(),
                            )),
                        }
                    }
                    _ => Err(IntentError::type_error(
                        "add_months() requires (timestamp: Int, months: Int)".to_string(),
                    )),
                }
            },
        },
    );

    // @ntnt add_years
    // @module std/time
    // @signature add_years(timestamp: Int, years: Int) -> Int
    // Adds years to a Unix timestamp with calendar-aware logic.
    //
    // Properly handles leap year boundaries. For example, Feb 29 of a leap
    // year + 1 year = Feb 28 (non-leap year). Preserves the time-of-day component.
    // @param timestamp Unix timestamp in seconds
    // @param years Number of years to add (negative to subtract)
    // @returns New Unix timestamp with years added
    // @see_also add_months, add_days, is_leap_year, diff
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example add_years(0, 1) => 31536000 ~ "Add 1 year to epoch"
    // @error TypeError ~ "add_years() requires (timestamp: Int, years: Int)" fix: "Pass two Int arguments"
    // @error RuntimeError ~ "Invalid timestamp" fix: "Ensure the timestamp is a valid Unix epoch value"
    // @error RuntimeError ~ "Invalid date after year addition" fix: "The resulting date is out of representable range"
    module.insert(
        "add_years".to_string(),
        Value::NativeFunction {
            name: "add_years".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                match (&args[0], &args[1]) {
                    (Value::Int(ts), Value::Int(years)) => {
                        let dt = DateTime::from_timestamp(*ts, 0).ok_or_else(|| {
                            IntentError::runtime_error("Invalid timestamp".to_string())
                        })?;

                        let new_year = dt.year() + (*years as i32);
                        let month = dt.month();
                        let day = dt.day();

                        // Handle Feb 29 on non-leap years
                        let new_day = if month == 2 && day == 29 {
                            let is_leap =
                                (new_year % 4 == 0 && new_year % 100 != 0) || (new_year % 400 == 0);
                            if is_leap {
                                29
                            } else {
                                28
                            }
                        } else {
                            day
                        };

                        match Utc.with_ymd_and_hms(
                            new_year,
                            month,
                            new_day,
                            dt.hour(),
                            dt.minute(),
                            dt.second(),
                        ) {
                            chrono::LocalResult::Single(new_dt) => {
                                Ok(Value::Int(new_dt.timestamp()))
                            }
                            _ => Err(IntentError::runtime_error(
                                "Invalid date after year addition".to_string(),
                            )),
                        }
                    }
                    _ => Err(IntentError::type_error(
                        "add_years() requires (timestamp: Int, years: Int)".to_string(),
                    )),
                }
            },
        },
    );

    // @ntnt diff
    // @module std/time
    // @signature diff(timestamp1: Int, timestamp2: Int) -> Map
    // Computes the difference between two timestamps.
    //
    // Returns a map with the difference expressed in multiple units:
    // { seconds, minutes, hours, days }. The result is timestamp1 - timestamp2,
    // so it is positive when timestamp1 is later.
    // @param timestamp1 First Unix timestamp in seconds
    // @param timestamp2 Second Unix timestamp in seconds
    // @returns Map with keys: seconds, minutes, hours, days (all Int)
    // @see_also before, after, equal
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example diff(86400, 0).days => 1 ~ "One day difference"
    // @example diff(3600, 0).hours => 1 ~ "One hour difference"
    // @error TypeError ~ "diff() requires two timestamps" fix: "Pass two Int arguments"
    module.insert(
        "diff".to_string(),
        Value::NativeFunction {
            name: "diff".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| match (&args[0], &args[1]) {
                (Value::Int(ts1), Value::Int(ts2)) => {
                    let diff_secs = ts1 - ts2;
                    let mut map = HashMap::new();
                    map.insert("seconds".to_string(), Value::Int(diff_secs));
                    map.insert("minutes".to_string(), Value::Int(diff_secs / 60));
                    map.insert("hours".to_string(), Value::Int(diff_secs / 3600));
                    map.insert("days".to_string(), Value::Int(diff_secs / 86400));
                    Ok(Value::Map(map))
                }
                _ => Err(IntentError::type_error(
                    "diff() requires two timestamps".to_string(),
                )),
            },
        },
    );

    // ========== Comparisons ==========

    // @ntnt before
    // @module std/time
    // @signature before(timestamp1: Int, timestamp2: Int) -> Bool
    // Checks whether the first timestamp is before the second.
    //
    // Returns true if timestamp1 < timestamp2.
    // @param timestamp1 First Unix timestamp in seconds
    // @param timestamp2 Second Unix timestamp in seconds
    // @returns true if timestamp1 is earlier than timestamp2
    // @see_also after, equal, diff
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example before(0, 86400) => true ~ "Epoch is before day 1"
    // @example before(86400, 0) => false ~ "Day 1 is not before epoch"
    // @error TypeError ~ "before() requires two timestamps" fix: "Pass two Int arguments"
    module.insert(
        "before".to_string(),
        Value::NativeFunction {
            name: "before".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| match (&args[0], &args[1]) {
                (Value::Int(ts1), Value::Int(ts2)) => Ok(Value::Bool(ts1 < ts2)),
                _ => Err(IntentError::type_error(
                    "before() requires two timestamps".to_string(),
                )),
            },
        },
    );

    // @ntnt after
    // @module std/time
    // @signature after(timestamp1: Int, timestamp2: Int) -> Bool
    // Checks whether the first timestamp is after the second.
    //
    // Returns true if timestamp1 > timestamp2.
    // @param timestamp1 First Unix timestamp in seconds
    // @param timestamp2 Second Unix timestamp in seconds
    // @returns true if timestamp1 is later than timestamp2
    // @see_also before, equal, diff
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example after(86400, 0) => true ~ "Day 1 is after epoch"
    // @example after(0, 86400) => false ~ "Epoch is not after day 1"
    // @error TypeError ~ "after() requires two timestamps" fix: "Pass two Int arguments"
    module.insert(
        "after".to_string(),
        Value::NativeFunction {
            name: "after".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| match (&args[0], &args[1]) {
                (Value::Int(ts1), Value::Int(ts2)) => Ok(Value::Bool(ts1 > ts2)),
                _ => Err(IntentError::type_error(
                    "after() requires two timestamps".to_string(),
                )),
            },
        },
    );

    // @ntnt equal
    // @module std/time
    // @signature equal(timestamp1: Int, timestamp2: Int) -> Bool
    // Checks whether two timestamps are equal.
    //
    // Returns true if both timestamps represent the same instant.
    // @param timestamp1 First Unix timestamp in seconds
    // @param timestamp2 Second Unix timestamp in seconds
    // @returns true if both timestamps are the same value
    // @see_also before, after, diff
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example equal(0, 0) => true ~ "Same timestamps are equal"
    // @example equal(0, 1) => false ~ "Different timestamps are not equal"
    // @error TypeError ~ "equal() requires two timestamps" fix: "Pass two Int arguments"
    module.insert(
        "equal".to_string(),
        Value::NativeFunction {
            name: "equal".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| match (&args[0], &args[1]) {
                (Value::Int(ts1), Value::Int(ts2)) => Ok(Value::Bool(ts1 == ts2)),
                _ => Err(IntentError::type_error(
                    "equal() requires two timestamps".to_string(),
                )),
            },
        },
    );

    // ========== Date Components ==========

    // @ntnt year
    // @module std/time
    // @signature year(timestamp: Int) -> Int
    // Extracts the year from a Unix timestamp (UTC).
    //
    // @param timestamp Unix timestamp in seconds
    // @returns The year as an integer (e.g., 2024)
    // @see_also month, day, hour, minute, second, to_utc
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example year(0) => 1970 ~ "Epoch year"
    // @error TypeError ~ "year() requires a timestamp" fix: "Pass an Int timestamp"
    // @error RuntimeError ~ "Invalid timestamp" fix: "Ensure the timestamp is a valid Unix epoch value"
    module.insert(
        "year".to_string(),
        Value::NativeFunction {
            name: "year".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::Int(ts) => {
                    let dt = DateTime::from_timestamp(*ts, 0).ok_or_else(|| {
                        IntentError::runtime_error("Invalid timestamp".to_string())
                    })?;
                    Ok(Value::Int(dt.year() as i64))
                }
                _ => Err(IntentError::type_error(
                    "year() requires a timestamp".to_string(),
                )),
            },
        },
    );

    // @ntnt month
    // @module std/time
    // @signature month(timestamp: Int) -> Int
    // Extracts the month from a Unix timestamp (UTC).
    //
    // @param timestamp Unix timestamp in seconds
    // @returns The month as an integer (1-12)
    // @see_also year, day, month_name, to_utc
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example month(0) => 1 ~ "Epoch is January"
    // @error TypeError ~ "month() requires a timestamp" fix: "Pass an Int timestamp"
    // @error RuntimeError ~ "Invalid timestamp" fix: "Ensure the timestamp is a valid Unix epoch value"
    module.insert(
        "month".to_string(),
        Value::NativeFunction {
            name: "month".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::Int(ts) => {
                    let dt = DateTime::from_timestamp(*ts, 0).ok_or_else(|| {
                        IntentError::runtime_error("Invalid timestamp".to_string())
                    })?;
                    Ok(Value::Int(dt.month() as i64))
                }
                _ => Err(IntentError::type_error(
                    "month() requires a timestamp".to_string(),
                )),
            },
        },
    );

    // @ntnt day
    // @module std/time
    // @signature day(timestamp: Int) -> Int
    // Extracts the day of the month from a Unix timestamp (UTC).
    //
    // @param timestamp Unix timestamp in seconds
    // @returns The day as an integer (1-31)
    // @see_also year, month, day_of_year, weekday, to_utc
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example day(0) => 1 ~ "Epoch is the 1st"
    // @error TypeError ~ "day() requires a timestamp" fix: "Pass an Int timestamp"
    // @error RuntimeError ~ "Invalid timestamp" fix: "Ensure the timestamp is a valid Unix epoch value"
    module.insert(
        "day".to_string(),
        Value::NativeFunction {
            name: "day".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::Int(ts) => {
                    let dt = DateTime::from_timestamp(*ts, 0).ok_or_else(|| {
                        IntentError::runtime_error("Invalid timestamp".to_string())
                    })?;
                    Ok(Value::Int(dt.day() as i64))
                }
                _ => Err(IntentError::type_error(
                    "day() requires a timestamp".to_string(),
                )),
            },
        },
    );

    // @ntnt hour
    // @module std/time
    // @signature hour(timestamp: Int) -> Int
    // Extracts the hour from a Unix timestamp (UTC).
    //
    // @param timestamp Unix timestamp in seconds
    // @returns The hour as an integer (0-23)
    // @see_also minute, second, year, month, day, to_utc
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example hour(0) => 0 ~ "Epoch is midnight"
    // @error TypeError ~ "hour() requires a timestamp" fix: "Pass an Int timestamp"
    // @error RuntimeError ~ "Invalid timestamp" fix: "Ensure the timestamp is a valid Unix epoch value"
    module.insert(
        "hour".to_string(),
        Value::NativeFunction {
            name: "hour".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::Int(ts) => {
                    let dt = DateTime::from_timestamp(*ts, 0).ok_or_else(|| {
                        IntentError::runtime_error("Invalid timestamp".to_string())
                    })?;
                    Ok(Value::Int(dt.hour() as i64))
                }
                _ => Err(IntentError::type_error(
                    "hour() requires a timestamp".to_string(),
                )),
            },
        },
    );

    // @ntnt minute
    // @module std/time
    // @signature minute(timestamp: Int) -> Int
    // Extracts the minute from a Unix timestamp (UTC).
    //
    // @param timestamp Unix timestamp in seconds
    // @returns The minute as an integer (0-59)
    // @see_also hour, second, to_utc
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example minute(0) => 0 ~ "Epoch minute is 0"
    // @error TypeError ~ "minute() requires a timestamp" fix: "Pass an Int timestamp"
    // @error RuntimeError ~ "Invalid timestamp" fix: "Ensure the timestamp is a valid Unix epoch value"
    module.insert(
        "minute".to_string(),
        Value::NativeFunction {
            name: "minute".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::Int(ts) => {
                    let dt = DateTime::from_timestamp(*ts, 0).ok_or_else(|| {
                        IntentError::runtime_error("Invalid timestamp".to_string())
                    })?;
                    Ok(Value::Int(dt.minute() as i64))
                }
                _ => Err(IntentError::type_error(
                    "minute() requires a timestamp".to_string(),
                )),
            },
        },
    );

    // @ntnt second
    // @module std/time
    // @signature second(timestamp: Int) -> Int
    // Extracts the second from a Unix timestamp (UTC).
    //
    // @param timestamp Unix timestamp in seconds
    // @returns The second as an integer (0-59)
    // @see_also hour, minute, to_utc
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example second(0) => 0 ~ "Epoch second is 0"
    // @error TypeError ~ "second() requires a timestamp" fix: "Pass an Int timestamp"
    // @error RuntimeError ~ "Invalid timestamp" fix: "Ensure the timestamp is a valid Unix epoch value"
    module.insert(
        "second".to_string(),
        Value::NativeFunction {
            name: "second".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::Int(ts) => {
                    let dt = DateTime::from_timestamp(*ts, 0).ok_or_else(|| {
                        IntentError::runtime_error("Invalid timestamp".to_string())
                    })?;
                    Ok(Value::Int(dt.second() as i64))
                }
                _ => Err(IntentError::type_error(
                    "second() requires a timestamp".to_string(),
                )),
            },
        },
    );

    // @ntnt weekday
    // @module std/time
    // @signature weekday(timestamp: Int) -> Int
    // Extracts the day of the week from a Unix timestamp (UTC).
    //
    // Returns 0 for Sunday, 1 for Monday, through 6 for Saturday.
    // @param timestamp Unix timestamp in seconds
    // @returns The weekday as an integer (0=Sunday, 6=Saturday)
    // @see_also weekday_name, day, day_of_year
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example weekday(0) => 4 ~ "Epoch (Jan 1, 1970) was a Thursday"
    // @error TypeError ~ "weekday() requires a timestamp" fix: "Pass an Int timestamp"
    // @error RuntimeError ~ "Invalid timestamp" fix: "Ensure the timestamp is a valid Unix epoch value"
    module.insert(
        "weekday".to_string(),
        Value::NativeFunction {
            name: "weekday".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::Int(ts) => {
                    let dt = DateTime::from_timestamp(*ts, 0).ok_or_else(|| {
                        IntentError::runtime_error("Invalid timestamp".to_string())
                    })?;
                    Ok(Value::Int(dt.weekday().num_days_from_sunday() as i64))
                }
                _ => Err(IntentError::type_error(
                    "weekday() requires a timestamp".to_string(),
                )),
            },
        },
    );

    // @ntnt weekday_name
    // @module std/time
    // @signature weekday_name(timestamp: Int) -> String
    // Returns the full English name of the weekday for a timestamp (UTC).
    //
    // Returns one of: "Sunday", "Monday", "Tuesday", "Wednesday",
    // "Thursday", "Friday", "Saturday".
    // @param timestamp Unix timestamp in seconds
    // @returns Full weekday name as a string
    // @see_also weekday, month_name, day
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example weekday_name(0) => "Thursday" ~ "Epoch was a Thursday"
    // @error TypeError ~ "weekday_name() requires a timestamp" fix: "Pass an Int timestamp"
    // @error RuntimeError ~ "Invalid timestamp" fix: "Ensure the timestamp is a valid Unix epoch value"
    module.insert(
        "weekday_name".to_string(),
        Value::NativeFunction {
            name: "weekday_name".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::Int(ts) => {
                    let dt = DateTime::from_timestamp(*ts, 0).ok_or_else(|| {
                        IntentError::runtime_error("Invalid timestamp".to_string())
                    })?;
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
                _ => Err(IntentError::type_error(
                    "weekday_name() requires a timestamp".to_string(),
                )),
            },
        },
    );

    // @ntnt month_name
    // @module std/time
    // @signature month_name(timestamp: Int) -> String
    // Returns the full English name of the month for a timestamp (UTC).
    //
    // Returns one of: "January" through "December".
    // @param timestamp Unix timestamp in seconds
    // @returns Full month name as a string
    // @see_also month, weekday_name, year
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example month_name(0) => "January" ~ "Epoch is in January"
    // @error TypeError ~ "month_name() requires a timestamp" fix: "Pass an Int timestamp"
    // @error RuntimeError ~ "Invalid timestamp" fix: "Ensure the timestamp is a valid Unix epoch value"
    module.insert(
        "month_name".to_string(),
        Value::NativeFunction {
            name: "month_name".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::Int(ts) => {
                    let dt = DateTime::from_timestamp(*ts, 0).ok_or_else(|| {
                        IntentError::runtime_error("Invalid timestamp".to_string())
                    })?;
                    let name = match dt.month() {
                        1 => "January",
                        2 => "February",
                        3 => "March",
                        4 => "April",
                        5 => "May",
                        6 => "June",
                        7 => "July",
                        8 => "August",
                        9 => "September",
                        10 => "October",
                        11 => "November",
                        12 => "December",
                        _ => "Unknown",
                    };
                    Ok(Value::String(name.to_string()))
                }
                _ => Err(IntentError::type_error(
                    "month_name() requires a timestamp".to_string(),
                )),
            },
        },
    );

    // @ntnt day_of_year
    // @module std/time
    // @signature day_of_year(timestamp: Int) -> Int
    // Extracts the ordinal day of the year from a timestamp (UTC).
    //
    // Returns the day number within the year (1-366).
    // @param timestamp Unix timestamp in seconds
    // @returns The ordinal day of the year (1-366)
    // @see_also day, weekday, is_leap_year
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example day_of_year(0) => 1 ~ "Epoch is day 1 of the year"
    // @error TypeError ~ "day_of_year() requires a timestamp" fix: "Pass an Int timestamp"
    // @error RuntimeError ~ "Invalid timestamp" fix: "Ensure the timestamp is a valid Unix epoch value"
    module.insert(
        "day_of_year".to_string(),
        Value::NativeFunction {
            name: "day_of_year".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::Int(ts) => {
                    let dt = DateTime::from_timestamp(*ts, 0).ok_or_else(|| {
                        IntentError::runtime_error("Invalid timestamp".to_string())
                    })?;
                    Ok(Value::Int(dt.ordinal() as i64))
                }
                _ => Err(IntentError::type_error(
                    "day_of_year() requires a timestamp".to_string(),
                )),
            },
        },
    );

    // @ntnt is_leap_year
    // @module std/time
    // @signature is_leap_year(timestamp: Int) -> Bool
    // Checks whether the year of the given timestamp is a leap year.
    //
    // Uses the standard Gregorian leap year rules: divisible by 4,
    // except centuries unless also divisible by 400.
    // @param timestamp Unix timestamp in seconds
    // @returns true if the timestamp falls in a leap year
    // @see_also year, day_of_year, add_years
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example is_leap_year(0) => false ~ "1970 is not a leap year"
    // @error TypeError ~ "is_leap_year() requires a timestamp" fix: "Pass an Int timestamp"
    // @error RuntimeError ~ "Invalid timestamp" fix: "Ensure the timestamp is a valid Unix epoch value"
    module.insert(
        "is_leap_year".to_string(),
        Value::NativeFunction {
            name: "is_leap_year".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::Int(ts) => {
                    let dt = DateTime::from_timestamp(*ts, 0).ok_or_else(|| {
                        IntentError::runtime_error("Invalid timestamp".to_string())
                    })?;
                    let year = dt.year();
                    let is_leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
                    Ok(Value::Bool(is_leap))
                }
                _ => Err(IntentError::type_error(
                    "is_leap_year() requires a timestamp".to_string(),
                )),
            },
        },
    );

    // ========== Utilities ==========

    // @ntnt sleep
    // @module std/time
    // @signature sleep(millis: Int) -> Unit
    // Pauses execution for the specified number of milliseconds.
    //
    // Blocks the current thread. Requires a non-negative value.
    // Use sparingly in production code; primarily useful for testing,
    // rate limiting, or animation delays.
    // @param millis Duration to sleep in milliseconds (must be >= 0)
    // @returns Unit
    // @see_also elapsed, now_millis
    // @since v0.1.0
    // @tags #time
    // @example sleep(100) => Unit ~ "Pauses for 100ms"
    // @error TypeError ~ "sleep() requires an integer (milliseconds)" fix: "Pass an Int value"
    // @error RuntimeError ~ "sleep() requires non-negative milliseconds" fix: "Pass a value >= 0"
    module.insert(
        "sleep".to_string(),
        Value::NativeFunction {
            name: "sleep".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::Int(ms) => {
                    if *ms < 0 {
                        return Err(IntentError::runtime_error(
                            "sleep() requires non-negative milliseconds".to_string(),
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(*ms as u64));
                    Ok(Value::Unit)
                }
                _ => Err(IntentError::type_error(
                    "sleep() requires an integer (milliseconds)".to_string(),
                )),
            },
        },
    );

    // @ntnt elapsed
    // @module std/time
    // @signature elapsed(start_millis: Int) -> Int
    // Returns the number of milliseconds elapsed since the given start time.
    //
    // Computes (now_millis() - start_millis). Useful for measuring
    // execution time of code blocks.
    // @param start_millis A starting timestamp in milliseconds (from now_millis())
    // @returns Milliseconds elapsed since start_millis
    // @see_also now_millis, now, sleep
    // @since v0.1.0
    // @tags #time
    // @example elapsed(now_millis()) => 0 ~ "Elapsed time is near 0 when called immediately"
    // @error TypeError ~ "elapsed() requires a start timestamp" fix: "Pass an Int value from now_millis()"
    module.insert(
        "elapsed".to_string(),
        Value::NativeFunction {
            name: "elapsed".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::Int(start) => {
                    let now = Utc::now().timestamp_millis();
                    Ok(Value::Int(now - start))
                }
                _ => Err(IntentError::type_error(
                    "elapsed() requires a start timestamp".to_string(),
                )),
            },
        },
    );

    // ========== Timezone Utilities ==========

    // @ntnt list_timezones
    // @module std/time
    // @signature list_timezones() -> Array<String>
    // Returns a list of commonly used IANA timezone identifiers.
    //
    // Provides a curated set of ~25 widely-used timezone strings that can
    // be passed to to_timezone() and format_in(). Includes major cities
    // across all continents.
    // @returns Array of IANA timezone identifier strings
    // @see_also to_timezone, format_in
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example list_timezones()[0] => "UTC" ~ "First timezone is UTC"
    module.insert(
        "list_timezones".to_string(),
        Value::NativeFunction {
            name: "list_timezones".to_string(),
            arity: 0,
            max_arity: 0,
            requires: None,
            func: |_args| {
                let common = vec![
                    "UTC",
                    "America/New_York",
                    "America/Chicago",
                    "America/Denver",
                    "America/Los_Angeles",
                    "America/Phoenix",
                    "America/Anchorage",
                    "Pacific/Honolulu",
                    "Europe/London",
                    "Europe/Paris",
                    "Europe/Berlin",
                    "Europe/Moscow",
                    "Asia/Tokyo",
                    "Asia/Shanghai",
                    "Asia/Singapore",
                    "Asia/Dubai",
                    "Asia/Kolkata",
                    "Australia/Sydney",
                    "Australia/Melbourne",
                    "Pacific/Auckland",
                    "America/Toronto",
                    "America/Vancouver",
                    "America/Mexico_City",
                    "America/Sao_Paulo",
                    "Africa/Cairo",
                    "Africa/Johannesburg",
                ];
                Ok(Value::Array(
                    common
                        .iter()
                        .map(|s| Value::String(s.to_string()))
                        .collect(),
                ))
            },
        },
    );

    // ========== Constants ==========

    // Duration constants in seconds
    module.insert("SECOND".to_string(), Value::Int(1));
    module.insert("MINUTE".to_string(), Value::Int(60));
    module.insert("HOUR".to_string(), Value::Int(3600));
    module.insert("DAY".to_string(), Value::Int(86400));
    module.insert("WEEK".to_string(), Value::Int(604800));

    // ========== Backward Compatibility ==========

    // @ntnt format_timestamp
    // @module std/time
    // @signature format_timestamp(timestamp: Int, format_str: String) -> String
    // Formats a Unix timestamp as a string (legacy alias for format()).
    //
    // Deprecated alias for format(). Retained for backward compatibility.
    // Prefer using format() in new code.
    // @param timestamp Unix timestamp in seconds
    // @param format_str strftime-compatible format string
    // @returns Formatted date/time string
    // @see_also format, format_in, to_iso
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example format_timestamp(0, "%Y-%m-%d") => "1970-01-01" ~ "Epoch date formatted"
    // @error TypeError ~ "format_timestamp() requires int and format string" fix: "Pass an Int timestamp and a String format"
    // @error RuntimeError ~ "Invalid timestamp" fix: "Ensure the timestamp is a valid Unix epoch value"
    module.insert(
        "format_timestamp".to_string(),
        Value::NativeFunction {
            name: "format_timestamp".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| match (&args[0], &args[1]) {
                (Value::Int(ts), Value::String(fmt)) => {
                    let dt = DateTime::from_timestamp(*ts, 0).ok_or_else(|| {
                        IntentError::runtime_error("Invalid timestamp".to_string())
                    })?;
                    Ok(Value::String(dt.format(fmt).to_string()))
                }
                _ => Err(IntentError::type_error(
                    "format_timestamp() requires int and format string".to_string(),
                )),
            },
        },
    );

    // @ntnt duration_secs
    // @module std/time
    // @signature duration_secs(seconds: Int) -> Map
    // Creates a duration map from seconds (legacy utility).
    //
    // Converts a duration in seconds to a map with keys: secs, millis, nanos.
    // Retained for backward compatibility. Prefer using the SECOND/MINUTE/HOUR
    // constants with arithmetic in new code.
    // @param seconds Duration in seconds
    // @returns Map with keys: secs (Int), millis (Int), nanos (Int)
    // @see_also duration_millis, SECOND, MINUTE, HOUR
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example duration_secs(1).millis => 1000 ~ "1 second = 1000 milliseconds"
    // @example duration_secs(1).nanos => 1000000000 ~ "1 second = 1 billion nanoseconds"
    // @error TypeError ~ "duration_secs() requires an integer" fix: "Pass an Int value"
    module.insert(
        "duration_secs".to_string(),
        Value::NativeFunction {
            name: "duration_secs".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::Int(secs) => {
                    let mut map = HashMap::new();
                    map.insert("secs".to_string(), Value::Int(*secs));
                    map.insert("millis".to_string(), Value::Int(*secs * 1000));
                    map.insert("nanos".to_string(), Value::Int(*secs * 1_000_000_000));
                    Ok(Value::Map(map))
                }
                _ => Err(IntentError::type_error(
                    "duration_secs() requires an integer".to_string(),
                )),
            },
        },
    );

    // @ntnt duration_millis
    // @module std/time
    // @signature duration_millis(milliseconds: Int) -> Map
    // Creates a duration map from milliseconds (legacy utility).
    //
    // Converts a duration in milliseconds to a map with keys: secs, millis, nanos.
    // Retained for backward compatibility. Prefer using the SECOND/MINUTE/HOUR
    // constants with arithmetic in new code.
    // @param milliseconds Duration in milliseconds
    // @returns Map with keys: secs (Int), millis (Int), nanos (Int)
    // @see_also duration_secs, SECOND, MINUTE, HOUR
    // @since v0.1.0
    // @tags #time #pure #deterministic
    // @example duration_millis(1000).secs => 1 ~ "1000 milliseconds = 1 second"
    // @example duration_millis(500).nanos => 500000000 ~ "500ms in nanoseconds"
    // @error TypeError ~ "duration_millis() requires an integer" fix: "Pass an Int value"
    module.insert(
        "duration_millis".to_string(),
        Value::NativeFunction {
            name: "duration_millis".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::Int(ms) => {
                    let mut map = HashMap::new();
                    map.insert("secs".to_string(), Value::Int(*ms / 1000));
                    map.insert("millis".to_string(), Value::Int(*ms));
                    map.insert("nanos".to_string(), Value::Int(*ms * 1_000_000));
                    Ok(Value::Map(map))
                }
                _ => Err(IntentError::type_error(
                    "duration_millis() requires an integer".to_string(),
                )),
            },
        },
    );

    // @ntnt from_now
    // @module std/time
    // @signature from_now(timestamp: Int) -> String
    // Human-friendly relative time: "3 hours ago", "in 2 days", "just now".
    //
    // Takes a Unix timestamp in seconds (as returned by now()) and renders
    // its distance from the current time. Past timestamps read "N units
    // ago", future ones "in N units"; anything within 60 seconds is
    // "just now". Units step through minutes, hours, days, months
    // (30-day) and years (365-day).
    // @param timestamp Unix timestamp in seconds.
    // @returns The relative description.
    // @see_also time_ago, now, diff, format
    // @since v0.4.12
    // @tags #time, #formatting
    // @example from_now(now() - 7200) => "2 hours ago" ~ "Past timestamp"
    // @example from_now(now() + 172800) => "in 2 days" ~ "Future timestamp"
    // @example from_now(now()) => "just now" ~ "Current moment"
    module.insert(
        "from_now".to_string(),
        Value::NativeFunction {
            name: "from_now".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| relative_time(&args[0], "from_now"),
        },
    );

    // @ntnt time_ago
    // @module std/time
    // @signature time_ago(timestamp: Int) -> String
    // Alias of from_now(): "3 hours ago", "in 2 days", "just now".
    //
    // Provided under the name most web developers reach for; identical
    // behavior to from_now().
    // @param timestamp Unix timestamp in seconds.
    // @returns The relative description.
    // @see_also from_now, now
    // @since v0.4.12
    // @tags #time, #formatting
    // @example time_ago(now() - 60) => "1 minute ago" ~ "One minute back"
    module.insert(
        "time_ago".to_string(),
        Value::NativeFunction {
            name: "time_ago".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| relative_time(&args[0], "time_ago"),
        },
    );

    module
}

/// Shared implementation for from_now()/time_ago(): render the distance
/// between a Unix-seconds timestamp and the current time
fn relative_time(value: &Value, fn_name: &str) -> Result<Value, IntentError> {
    let timestamp = match value {
        Value::Int(n) => *n,
        Value::Float(f) => *f as i64,
        other => {
            return Err(IntentError::type_error(format!(
                "{}() requires a Unix timestamp in seconds, got {}",
                fn_name,
                other.type_name()
            )))
        }
    };

    let now = chrono::Utc::now().timestamp();
    let delta = timestamp - now;
    let past = delta <= 0;
    let magnitude = delta.unsigned_abs();

    let (count, unit) = if magnitude < 60 {
        return Ok(Value::String("just now".to_string()));
    } else if magnitude < 3600 {
        (magnitude / 60, "minute")
    } else if magnitude < 86_400 {
        (magnitude / 3600, "hour")
    } else if magnitude < 30 * 86_400 {
        (magnitude / 86_400, "day")
    } else if magnitude < 365 * 86_400 {
        (magnitude / (30 * 86_400), "month")
    } else {
        (magnitude / (365 * 86_400), "year")
    };

    let plural = if count == 1 { "" } else { "s" };
    let rendered = if past {
        format!("{} {}{} ago", count, unit, plural)
    } else {
        format!("in {} {}{}", count, unit, plural)
    };
    Ok(Value::String(rendered))
}
