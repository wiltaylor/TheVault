---
id: '87'
title: Putting a device script before the device file that names it reports false
  compile problems that never clear
blocked_by: []
status: todo
verified: unknown
---

# Putting a device script before the device file that names it reports false compile problems that never clear

Putting a device script before the device file that names it reports false problems. Repro on a vault with no devices/ file: `PUT /api/scripts/device_messages` with the shipped source returns problems `[{"role":"task","message":"script \"device_messages\" does not compile: [E0200] unknown module `device` ..."}]`. After `PUT /api/device-files/android` (which names the script), putting the same source again returns `problems: []`. The script is checked as a task script because no device file names it yet, and putting the device file does not re-check the scripts it names, so the stale problems stay wherever problems are stored or shown.

Fix: when a device file is put, re-check every script it names as a device script and replace their recorded problems; when a script is put that no file names yet and it uses the `device` module, report one clear problem ("uses the device module but no devices/<platform>.wcl names it") rather than a list of compile errors. Check the web client's script editor and Settings > Devices show the current problems after either order.

Acceptance criteria:
- Putting the five device scripts and then devices/android.wcl leaves no recorded problems on any of them (API test).
- A script using `device` that no device file names reports exactly the one clear problem.
- `./dev just check` passes.

## Worklog
