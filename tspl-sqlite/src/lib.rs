//! The tspl-sqlite crate provides a common set of functions
//! for initializing, migrating and managing SQLite databases.
//!
//! It also includes some traits used to make common DB operations
//! (such as SELECT) easier.

pub mod errors;
pub mod traits;
pub mod migrations;

use self::errors::Result;
use rusqlite::Connection;
use self::migrations::Migration;
use std::path::{PathBuf, Path};

fn initialize_connection_without_migrating(conn: &mut Connection) -> ::std::result::Result<(), rusqlite::Error> {
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    conn.execute_batch("PRAGMA journal_mode = WAL;")?;
    conn.busy_timeout(std::time::Duration::new(5, 0))?;
    Ok(())
}
pub fn initialize_connection(conn: &mut Connection, migrations: &[Migration]) -> Result<()> {
    initialize_connection_without_migrating(conn)?;
    migrations::initialize_migrations(conn)?;
    migrations::run_pending_migrations(conn, migrations)?;
    Ok(())
}
pub fn initialize_db<P: AsRef<Path>>(path: P, migrations: &[Migration]) -> Result<Connection> {
    let mut conn = Connection::open(path)?;
    initialize_connection(&mut conn, migrations)?;
    Ok(conn)
}

pub struct TsplConnectionManager {
    path: PathBuf
}
impl TsplConnectionManager {
    pub fn initialize<P: AsRef<Path>>(path: P, migrations: &[Migration]) -> Result<Self> {
        let p: PathBuf = path.as_ref().to_owned();
        let _conn = initialize_db(path, migrations)?;
        Ok(Self { path: p })
    }
}
impl r2d2::ManageConnection for TsplConnectionManager {
    type Connection = Connection;
    type Error = rusqlite::Error; // annoyingly this can't just be SqlError, because stupid Error trait constraint :(

    fn connect(&self) -> ::std::result::Result<Connection, rusqlite::Error> {
        let mut conn = Connection::open(&self.path)?;
        initialize_connection_without_migrating(&mut conn)?;
        Ok(conn)
    }

    fn is_valid(&self, conn: &mut Connection) -> ::std::result::Result<(), rusqlite::Error> {
        conn.execute_batch("").map_err(Into::into)
    }

    fn has_broken(&self, _: &mut Connection) -> bool {
        false
    }
}
pub type TsplPool = r2d2::Pool<TsplConnectionManager>;

pub use rusqlite;
pub use uuid;
pub use r2d2;
