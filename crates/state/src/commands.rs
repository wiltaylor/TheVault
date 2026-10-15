//! Commands server scripts queue for a device, such as sending an SMS (D22).
//!
//! The server checks that a command is declared and enabled before it queues it; this module
//! stores what was queued and what became of it. A command is `pending` until the device reports
//! on it: `delivered` once the device has it, then `done` or `failed` with a result. A command the
//! device has not finished by `expires_ms` reads as `expired` and can no longer be finished.
//! Finished and expired commands are kept for [`RETENTION_MS`].

use rusqlite::{OptionalExtension, Row, params};

use crate::{Error, LiveEvent, Result, State};

/// How long a finished or expired command is kept, in milliseconds: 30 days.
pub const RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// Where a command is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandStatus {
    Pending,
    Delivered,
    Done,
    Failed,
    Expired,
}

impl CommandStatus {
    pub const ALL: [CommandStatus; 5] = [
        CommandStatus::Pending,
        CommandStatus::Delivered,
        CommandStatus::Done,
        CommandStatus::Failed,
        CommandStatus::Expired,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            CommandStatus::Pending => "pending",
            CommandStatus::Delivered => "delivered",
            CommandStatus::Done => "done",
            CommandStatus::Failed => "failed",
            CommandStatus::Expired => "expired",
        }
    }

    pub fn parse(s: &str) -> Option<CommandStatus> {
        CommandStatus::ALL.into_iter().find(|c| c.as_str() == s)
    }

    /// Whether the device may still report on the command.
    pub fn is_open(self) -> bool {
        matches!(self, CommandStatus::Pending | CommandStatus::Delivered)
    }
}

/// A command to queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCommand<'a> {
    pub device_id: &'a str,
    pub name: &'a str,
    /// JSON text.
    pub payload: &'a str,
    /// Who queued it, such as `task:<name>` or `handler:<type>.<event>`.
    pub source: &'a str,
    /// How long the device has to finish it, from now.
    pub ttl_ms: i64,
}

/// One queued command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCommand {
    /// A ULID.
    pub id: String,
    pub device_id: String,
    pub name: String,
    /// JSON text.
    pub payload: String,
    pub created_ms: i64,
    pub expires_ms: i64,
    pub status: CommandStatus,
    /// JSON text the device reported with `done` or `failed`.
    pub result: Option<String>,
    pub source: String,
    /// When the device last reported on it.
    pub updated_ms: Option<i64>,
}

const COLUMNS: &str =
    "id, device_id, name, payload, created_ms, expires_ms, status, result, source, updated_ms";

fn command_row(row: &Row<'_>) -> rusqlite::Result<DeviceCommand> {
    let status: String = row.get(6)?;
    Ok(DeviceCommand {
        id: row.get(0)?,
        device_id: row.get(1)?,
        name: row.get(2)?,
        payload: row.get(3)?,
        created_ms: row.get(4)?,
        expires_ms: row.get(5)?,
        status: CommandStatus::parse(&status).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                6,
                rusqlite::types::Type::Text,
                format!("unknown command status {status:?}").into(),
            )
        })?,
        result: row.get(7)?,
        source: row.get(8)?,
        updated_ms: row.get(9)?,
    })
}

impl State {
    /// Mark every open command past its expiry as expired.
    fn expire_commands(&self, conn: &rusqlite::Connection) -> Result<()> {
        conn.execute(
            "UPDATE device_commands SET status = 'expired'
             WHERE status IN ('pending', 'delivered') AND expires_ms <= ?1",
            [self.now()],
        )?;
        Ok(())
    }

    /// Queue a command for a device and tell open clients. Commands finished or expired longer
    /// than [`RETENTION_MS`] ago are removed on the way.
    pub fn queue_device_command(&self, new: &NewCommand<'_>) -> Result<DeviceCommand> {
        self.device(new.device_id)?;
        let id = self.new_ulid().to_string();
        let now = self.now();
        let conn = self.conn();
        self.expire_commands(&conn)?;
        conn.execute(
            "DELETE FROM device_commands
             WHERE status NOT IN ('pending', 'delivered')
               AND COALESCE(updated_ms, expires_ms) <= ?1",
            [now - RETENTION_MS],
        )?;
        conn.execute(
            "INSERT INTO device_commands
                 (id, device_id, name, payload, created_ms, expires_ms, status, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7)",
            params![
                id,
                new.device_id,
                new.name,
                new.payload,
                now,
                now + new.ttl_ms,
                new.source
            ],
        )?;
        self.events.publish(LiveEvent::DeviceCommand {
            device: new.device_id.to_owned(),
            id: id.clone(),
        });
        drop(conn);
        self.device_command(new.device_id, &id)
    }

    /// A device's commands in the order they were queued, only those with `status` when it is
    /// given.
    pub fn device_commands(
        &self,
        device_id: &str,
        status: Option<CommandStatus>,
    ) -> Result<Vec<DeviceCommand>> {
        self.device(device_id)?;
        let conn = self.conn();
        self.expire_commands(&conn)?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM device_commands
             WHERE device_id = ?1 AND (?2 IS NULL OR status = ?2) ORDER BY created_ms, rowid"
        ))?;
        let rows = stmt.query_map(
            params![device_id, status.map(CommandStatus::as_str)],
            command_row,
        )?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// One of a device's commands.
    pub fn device_command(&self, device_id: &str, id: &str) -> Result<DeviceCommand> {
        let conn = self.conn();
        self.expire_commands(&conn)?;
        conn.query_row(
            &format!("SELECT {COLUMNS} FROM device_commands WHERE device_id = ?1 AND id = ?2"),
            [device_id, id],
            command_row,
        )
        .optional()?
        .ok_or_else(|| Error::UnknownCommand(id.to_owned()))
    }

    /// Record what the device reports on a command: `delivered`, or `done` or `failed` with
    /// `result` (JSON text). A command already finished or expired is refused with
    /// [`Error::CommandClosed`].
    pub fn report_device_command(
        &self,
        device_id: &str,
        id: &str,
        status: CommandStatus,
        result: Option<&str>,
    ) -> Result<DeviceCommand> {
        if !matches!(
            status,
            CommandStatus::Delivered | CommandStatus::Done | CommandStatus::Failed
        ) {
            return Err(Error::InvalidCommandReport(format!(
                "a device reports delivered, done or failed, not {}",
                status.as_str()
            )));
        }
        let current = self.device_command(device_id, id)?;
        if !current.status.is_open() {
            return Err(Error::CommandClosed {
                id: id.to_owned(),
                status: current.status.as_str(),
            });
        }
        let changed = self.conn().execute(
            "UPDATE device_commands SET status = ?3, result = COALESCE(?4, result), updated_ms = ?5
             WHERE device_id = ?1 AND id = ?2 AND status IN ('pending', 'delivered')",
            params![device_id, id, status.as_str(), result, self.now()],
        )?;
        if changed == 0 {
            // It expired or was finished between the read and the write.
            let now = self.device_command(device_id, id)?;
            return Err(Error::CommandClosed {
                id: id.to_owned(),
                status: now.status.as_str(),
            });
        }
        self.device_command(device_id, id)
    }
}
