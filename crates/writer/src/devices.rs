//! Device files, `devices/<platform>.wcl` (D5), and the commands server scripts queue for a
//! device through `vault::device_command` (D22).
//!
//! A command is queued only when the device's platform file declares it and the user enabled it
//! for that device in Settings > Devices, which the device's `settings.commands` records.

use schema::{DEVICE_PLATFORMS, DeviceFile, Invalid, Issue};
use state::{DeviceCommand, NewCommand};
use store::Change;

use crate::{Error, Result, Writer};

/// How long a device has to finish a command before it expires: one day.
pub const COMMAND_TTL_MS: i64 = 24 * 60 * 60 * 1000;

/// The largest command payload, as JSON text.
pub const MAX_COMMAND_PAYLOAD: usize = 64 * 1024;

/// The vault path of the device file for `platform`.
pub fn device_file_path(platform: &str) -> String {
    format!("devices/{platform}.wcl")
}

fn check_platform(platform: &str) -> Result<()> {
    if DEVICE_PLATFORMS.contains(&platform) {
        Ok(())
    } else {
        Err(Error::BadRequest(format!(
            "{platform:?} is not a device platform: {}",
            DEVICE_PLATFORMS.join(", ")
        )))
    }
}

fn missing_device_file(platform: &str) -> Error {
    Error::Missing(format!("there is no device file for {platform:?}"))
}

impl Writer {
    /// The `(platform, text)` of every `devices/<platform>.wcl`, sorted by platform.
    pub fn device_files(&self) -> Result<Vec<(String, String)>> {
        let mut files: Vec<_> = self
            .definitions("devices")?
            .into_iter()
            .filter(|(p, _)| DEVICE_PLATFORMS.contains(&p.as_str()))
            .collect();
        files.sort();
        Ok(files)
    }

    /// The text of `devices/<platform>.wcl`.
    pub fn read_device_file(&self, platform: &str) -> Result<String> {
        check_platform(platform)?;
        match self.vault.read_text(&device_file_path(platform)) {
            Err(store::Error::NotFound(_)) => Err(missing_device_file(platform)),
            other => Ok(other?),
        }
    }

    /// Create or replace `devices/<platform>.wcl` in canonical form. Its events and commands must
    /// come from the closed lists, and every script it names must exist.
    pub fn put_device_file(&self, platform: &str, source: &str) -> Result<String> {
        let _w = self.write_lock();
        let mut inner = self.lock();
        check_platform(platform)?;
        let path = device_file_path(platform);
        let text = store::canonical(source, &path)?;
        let file = DeviceFile::parse(platform, &text).map_err(Error::Invalid)?;
        let missing: Vec<Issue> = file
            .scripts()
            .into_iter()
            .filter(|s| self.read_script(s).is_err())
            .map(|s| Issue::new("script", format!("there is no script {s:?}")))
            .collect();
        if !missing.is_empty() {
            return Err(Error::Invalid(Invalid {
                file: path,
                issues: missing,
            }));
        }
        let generation = self.commit(vec![Change::write(path, text.as_bytes())])?;
        inner.index.advance(generation)?;
        Ok(text)
    }

    /// Delete `devices/<platform>.wcl`. Devices of the platform then run no handlers and accept
    /// no commands.
    pub fn delete_device_file(&self, platform: &str) -> Result<()> {
        let _w = self.write_lock();
        let mut inner = self.lock();
        self.read_device_file(platform)?;
        let generation = self.commit(vec![Change::delete(device_file_path(platform))])?;
        inner.index.advance(generation)?;
        Ok(())
    }

    /// The device files that name script `name`, as `devices/<platform>.wcl (handler "<event>")`
    /// and `devices/<platform>.wcl (command "<name>")`.
    pub(crate) fn device_files_naming(&self, name: &str) -> Result<Vec<String>> {
        let mut by = Vec::new();
        for (platform, text) in self.device_files()? {
            let Ok(file) = DeviceFile::parse(&platform, &text) else {
                continue;
            };
            let path = device_file_path(&platform);
            for h in file.handlers.iter().filter(|h| h.script == name) {
                by.push(format!("{path} (handler {:?})", h.name));
            }
            for c in file.commands.iter().filter(|c| c.script == name) {
                by.push(format!("{path} (command {:?})", c.name));
            }
        }
        Ok(by)
    }

    /// Queue command `name` with `payload` for `device`, a device id or a device's name, on
    /// behalf of `source`. Refused unless the device's platform file declares the command and the
    /// device has it enabled.
    pub fn device_command(
        &self,
        device: &str,
        name: &str,
        payload: &serde_json::Value,
        source: &str,
    ) -> Result<DeviceCommand> {
        let state = self.state();
        let device = match state.device(device) {
            Ok(d) => d,
            Err(state::Error::UnknownDevice(_)) => {
                let mut named: Vec<_> = state
                    .list_devices()?
                    .into_iter()
                    .filter(|d| d.name == device)
                    .collect();
                match named.len() {
                    1 => named.remove(0),
                    0 => return Err(Error::BadRequest(format!("there is no device {device:?}"))),
                    n => {
                        return Err(Error::BadRequest(format!(
                            "{n} devices are named {device:?}; name the device by its id"
                        )));
                    }
                }
            }
            Err(e) => return Err(e.into()),
        };
        let path = device_file_path(&device.platform);
        let file = match self.vault.read_text(&path) {
            Ok(text) => DeviceFile::parse(&device.platform, &text).map_err(Error::Invalid)?,
            Err(store::Error::NotFound(_)) => {
                return Err(Error::BadRequest(format!(
                    "device {:?} accepts no commands: there is no {path}",
                    device.name
                )));
            }
            Err(e) => return Err(e.into()),
        };
        if file.command(name).is_none() {
            return Err(Error::BadRequest(format!(
                "device {:?} does not accept command {name:?}: {path} does not declare it",
                device.name
            )));
        }
        let enabled = serde_json::from_str::<serde_json::Value>(&device.settings)
            .ok()
            .and_then(|s| {
                s.get("commands")?
                    .as_array()
                    .map(|c| c.iter().any(|c| c == name))
            })
            .unwrap_or(false);
        if !enabled {
            return Err(Error::BadRequest(format!(
                "command {name:?} is not enabled for device {:?}; enable it in Settings > Devices",
                device.name
            )));
        }
        let payload = payload.to_string();
        if payload.len() > MAX_COMMAND_PAYLOAD {
            return Err(Error::BadRequest(format!(
                "the payload is larger than {MAX_COMMAND_PAYLOAD} bytes"
            )));
        }
        Ok(state.queue_device_command(&NewCommand {
            device_id: &device.id,
            name,
            payload: &payload,
            source,
            ttl_ms: COMMAND_TTL_MS,
        })?)
    }
}

/// A command in the JSON form the API and scripts see. `payload` and `result` are JSON values.
pub fn command_json(c: &DeviceCommand) -> serde_json::Value {
    let parse = |text: &str| serde_json::from_str(text).unwrap_or(serde_json::Value::Null);
    serde_json::json!({
        "id": c.id,
        "device": c.device_id,
        "name": c.name,
        "payload": parse(&c.payload),
        "status": c.status.as_str(),
        "result": c.result.as_deref().map(parse),
        "source": c.source,
        "created": c.created_ms,
        "expires": c.expires_ms,
        "updated": c.updated_ms,
    })
}
