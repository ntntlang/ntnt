//! What a SQLite failure may say about the user's own SQL (#424): the
//! extended result code's name, and SQLite's own message only in shapes that
//! hold fixed words and identifiers. Every variable part of such a shape is
//! checked to be a name, so a value from a failing row never passes.

/// The name sqlite3.h gives (extended) result code `code`, or the name of
/// its primary code when the extended code is not one this build knows.
pub(super) fn name(code: i32) -> &'static str {
    match code {
        0 => "SQLITE_OK",
        1 => "SQLITE_ERROR",
        2 => "SQLITE_INTERNAL",
        3 => "SQLITE_PERM",
        4 => "SQLITE_ABORT",
        5 => "SQLITE_BUSY",
        6 => "SQLITE_LOCKED",
        7 => "SQLITE_NOMEM",
        8 => "SQLITE_READONLY",
        9 => "SQLITE_INTERRUPT",
        10 => "SQLITE_IOERR",
        11 => "SQLITE_CORRUPT",
        12 => "SQLITE_NOTFOUND",
        13 => "SQLITE_FULL",
        14 => "SQLITE_CANTOPEN",
        15 => "SQLITE_PROTOCOL",
        16 => "SQLITE_EMPTY",
        17 => "SQLITE_SCHEMA",
        18 => "SQLITE_TOOBIG",
        19 => "SQLITE_CONSTRAINT",
        20 => "SQLITE_MISMATCH",
        21 => "SQLITE_MISUSE",
        22 => "SQLITE_NOLFS",
        23 => "SQLITE_AUTH",
        24 => "SQLITE_FORMAT",
        25 => "SQLITE_RANGE",
        26 => "SQLITE_NOTADB",
        27 => "SQLITE_NOTICE",
        28 => "SQLITE_WARNING",
        100 => "SQLITE_ROW",
        101 => "SQLITE_DONE",
        256 => "SQLITE_OK_LOAD_PERMANENTLY",
        257 => "SQLITE_ERROR_MISSING_COLLSEQ",
        261 => "SQLITE_BUSY_RECOVERY",
        262 => "SQLITE_LOCKED_SHAREDCACHE",
        264 => "SQLITE_READONLY_RECOVERY",
        266 => "SQLITE_IOERR_READ",
        267 => "SQLITE_CORRUPT_VTAB",
        270 => "SQLITE_CANTOPEN_NOTEMPDIR",
        275 => "SQLITE_CONSTRAINT_CHECK",
        279 => "SQLITE_AUTH_USER",
        283 => "SQLITE_NOTICE_RECOVER_WAL",
        284 => "SQLITE_WARNING_AUTOINDEX",
        512 => "SQLITE_OK_SYMLINK",
        513 => "SQLITE_ERROR_RETRY",
        516 => "SQLITE_ABORT_ROLLBACK",
        517 => "SQLITE_BUSY_SNAPSHOT",
        518 => "SQLITE_LOCKED_VTAB",
        520 => "SQLITE_READONLY_CANTLOCK",
        522 => "SQLITE_IOERR_SHORT_READ",
        523 => "SQLITE_CORRUPT_SEQUENCE",
        526 => "SQLITE_CANTOPEN_ISDIR",
        531 => "SQLITE_CONSTRAINT_COMMITHOOK",
        539 => "SQLITE_NOTICE_RECOVER_ROLLBACK",
        769 => "SQLITE_ERROR_SNAPSHOT",
        773 => "SQLITE_BUSY_TIMEOUT",
        776 => "SQLITE_READONLY_ROLLBACK",
        778 => "SQLITE_IOERR_WRITE",
        779 => "SQLITE_CORRUPT_INDEX",
        782 => "SQLITE_CANTOPEN_FULLPATH",
        787 => "SQLITE_CONSTRAINT_FOREIGNKEY",
        795 => "SQLITE_NOTICE_RBU",
        1025 => "SQLITE_ERROR_RESERVESIZE",
        1032 => "SQLITE_READONLY_DBMOVED",
        1034 => "SQLITE_IOERR_FSYNC",
        1038 => "SQLITE_CANTOPEN_CONVPATH",
        1043 => "SQLITE_CONSTRAINT_FUNCTION",
        1281 => "SQLITE_ERROR_KEY",
        1288 => "SQLITE_READONLY_CANTINIT",
        1290 => "SQLITE_IOERR_DIR_FSYNC",
        1294 => "SQLITE_CANTOPEN_DIRTYWAL",
        1299 => "SQLITE_CONSTRAINT_NOTNULL",
        1537 => "SQLITE_ERROR_UNABLE",
        1544 => "SQLITE_READONLY_DIRECTORY",
        1546 => "SQLITE_IOERR_TRUNCATE",
        1550 => "SQLITE_CANTOPEN_SYMLINK",
        1555 => "SQLITE_CONSTRAINT_PRIMARYKEY",
        1802 => "SQLITE_IOERR_FSTAT",
        1811 => "SQLITE_CONSTRAINT_TRIGGER",
        2058 => "SQLITE_IOERR_UNLOCK",
        2067 => "SQLITE_CONSTRAINT_UNIQUE",
        2314 => "SQLITE_IOERR_RDLOCK",
        2323 => "SQLITE_CONSTRAINT_VTAB",
        2570 => "SQLITE_IOERR_DELETE",
        2579 => "SQLITE_CONSTRAINT_ROWID",
        2826 => "SQLITE_IOERR_BLOCKED",
        2835 => "SQLITE_CONSTRAINT_PINNED",
        3082 => "SQLITE_IOERR_NOMEM",
        3091 => "SQLITE_CONSTRAINT_DATATYPE",
        3338 => "SQLITE_IOERR_ACCESS",
        3594 => "SQLITE_IOERR_CHECKRESERVEDLOCK",
        3850 => "SQLITE_IOERR_LOCK",
        4106 => "SQLITE_IOERR_CLOSE",
        4362 => "SQLITE_IOERR_DIR_CLOSE",
        4618 => "SQLITE_IOERR_SHMOPEN",
        4874 => "SQLITE_IOERR_SHMSIZE",
        5130 => "SQLITE_IOERR_SHMLOCK",
        5386 => "SQLITE_IOERR_SHMMAP",
        5642 => "SQLITE_IOERR_SEEK",
        5898 => "SQLITE_IOERR_DELETE_NOENT",
        6154 => "SQLITE_IOERR_MMAP",
        6410 => "SQLITE_IOERR_GETTEMPPATH",
        6666 => "SQLITE_IOERR_CONVPATH",
        6922 => "SQLITE_IOERR_VNODE",
        7178 => "SQLITE_IOERR_AUTH",
        7434 => "SQLITE_IOERR_BEGIN_ATOMIC",
        7690 => "SQLITE_IOERR_COMMIT_ATOMIC",
        7946 => "SQLITE_IOERR_ROLLBACK_ATOMIC",
        8202 => "SQLITE_IOERR_DATA",
        8458 => "SQLITE_IOERR_CORRUPTFS",
        8714 => "SQLITE_IOERR_IN_PAGE",
        8970 => "SQLITE_IOERR_BADKEY",
        9226 => "SQLITE_IOERR_CODEC",
        _ if code > 0xff => name(code & 0xff),
        _ => "an unknown SQLite result code",
    }
}

/// SQLite's `message` for result code `code`, when its shape is one whose
/// variable parts are identifiers from the schema or the migration: a
/// missing or duplicate object, or which constraint failed. `None` keeps it
/// out of the diagnostic.
pub(super) fn detail(code: i32, message: &str) -> Option<String> {
    use rusqlite::ffi;
    let shown = |ok: bool| ok.then(|| message.to_owned());
    match code {
        ffi::SQLITE_ERROR => error(message),
        ffi::SQLITE_CONSTRAINT_UNIQUE | ffi::SQLITE_CONSTRAINT_PRIMARYKEY => {
            let columns = message.strip_prefix("UNIQUE constraint failed: ")?;
            shown(
                columns.split(", ").all(qualified)
                    || columns
                        .strip_prefix("index '")
                        .and_then(|index| index.strip_suffix('\''))
                        .is_some_and(identifier),
            )
        }
        ffi::SQLITE_CONSTRAINT_NOTNULL => shown(
            message
                .strip_prefix("NOT NULL constraint failed: ")
                .is_some_and(qualified),
        ),
        // A named CHECK is reported by its name, an unnamed one by its
        // expression, which is migration text rather than a name.
        ffi::SQLITE_CONSTRAINT_CHECK => {
            Some(match message.strip_prefix("CHECK constraint failed: ") {
                Some(name) if identifier(name) => message.to_owned(),
                _ => "CHECK constraint failed".to_owned(),
            })
        }
        ffi::SQLITE_CONSTRAINT_FOREIGNKEY => shown(message == "FOREIGN KEY constraint failed"),
        // `cannot store TEXT value in INTEGER column t.b`: types, not values.
        ffi::SQLITE_CONSTRAINT_DATATYPE => {
            let rest = message.strip_prefix("cannot store ")?;
            let (stored, rest) = rest.split_once(" value in ")?;
            let (declared, column) = rest.split_once(" column ")?;
            shown(identifier(stored) && identifier(declared) && qualified(column))
        }
        ffi::SQLITE_MISMATCH => shown(message == "datatype mismatch"),
        ffi::SQLITE_TOOBIG => shown(message == "string or blob too big"),
        _ => None,
    }
}

/// The SQLITE_ERROR messages that name only objects of the schema.
fn error(message: &str) -> Option<String> {
    const FIXED: &[&str] = &[
        "incomplete input",
        "integer overflow",
        "Cannot add a PRIMARY KEY column",
        "Cannot add a UNIQUE column",
        "Cannot add a column with non-constant default",
        "Cannot add a NOT NULL column with default value NULL",
        "Cannot add a REFERENCES column with non-NULL default value",
        "cannot add a STORED column",
    ];
    const NAMED: &[&str] = &[
        "no such column: ",
        "no such table: ",
        "no such view: ",
        "no such index: ",
        "no such trigger: ",
        "ambiguous column name: ",
        "duplicate column name: ",
        "there is already another table or index with this name: ",
        "object name reserved for internal use: ",
    ];
    let shown = |ok: bool| ok.then(|| message.to_owned());
    if FIXED.contains(&message) {
        return Some(message.to_owned());
    }
    if let Some(name) = NAMED.iter().find_map(|prefix| message.strip_prefix(prefix)) {
        return shown(qualified(name));
    }
    if let Some(function) = message.strip_prefix("no such function: ") {
        return shown(identifier(function));
    }
    if let Some((kind, object)) = message
        .strip_suffix(" already exists")
        .and_then(|rest| rest.split_once(' '))
    {
        return shown(matches!(kind, "table" | "view" | "index" | "trigger") && qualified(object));
    }
    if let Some((table, column)) = message
        .strip_prefix("table ")
        .and_then(|rest| rest.split_once(" has no column named "))
    {
        return shown(qualified(table) && identifier(column));
    }
    if let Some((child, parent)) = message
        .strip_prefix("foreign key mismatch - ")
        .and_then(|rest| rest.split_once(" referencing "))
    {
        return shown(quoted(child) && quoted(parent));
    }
    if let Some(token) = message
        .strip_prefix("near \"")
        .and_then(|rest| rest.strip_suffix("\": syntax error"))
    {
        return shown(identifier(token) || symbol(token));
    }
    // ALTER TABLE checks the objects that depend on what it changes.
    if let Some((head, inner)) = message
        .strip_prefix("error in ")
        .and_then(|rest| rest.split_once(": "))
    {
        let (kind, object) = head.split_once(' ')?;
        if matches!(kind, "trigger" | "view" | "index") && qualified(object) {
            return error(inner).map(|inner| format!("error in {kind} {object}: {inner}"));
        }
    }
    None
}

/// A plain SQL name: ASCII letters, digits, `_` and `$`, not starting with
/// a digit. A literal never has this shape.
fn identifier(text: &str) -> bool {
    let mut chars = text.chars();
    text.len() <= 128
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}
/// `name`, `table.column` or `schema.table.column`.
fn qualified(text: &str) -> bool {
    let parts: Vec<&str> = text.split('.').collect();
    parts.len() <= 3 && parts.iter().all(|part| identifier(part))
}
/// A name in double quotes, as `foreign key mismatch` prints it.
fn quoted(text: &str) -> bool {
    text.strip_prefix('"')
        .and_then(|text| text.strip_suffix('"'))
        .is_some_and(identifier)
}
/// SQL punctuation, such as `;` or `)`.
fn symbol(text: &str) -> bool {
    !text.is_empty() && text.len() <= 2 && text.chars().all(|c| "(),;.=<>!+-*/%|&".contains(c))
}
