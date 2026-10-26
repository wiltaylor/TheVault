//! Agents using skills (skills S6, S7): the "Available skills" section of a turn's system text,
//! `use_skill`, `read_skill_file` and `install_skill_in_browser`.
//!
//! An agent is offered the valid skills its `permissions.skills` names, or every valid skill when
//! the list is absent ([`offered`]); a skill with errors is offered to no agent. The list is the
//! spec's metadata stage: each skill's name and description, in a section that ends the system
//! text ([`Snapshot::section`]). A thread keeps the section it took on its first turn in its
//! transcript and sends it unchanged on every later turn, so adding or changing skills never
//! changes a running thread's system text and prompt caching holds. A turn sent with
//! `update_skills` (the chat box's `/update-skills`) takes a fresh one; its user message is
//! [`UPDATED_TEXT`], with a [`UPDATED_NOTICE`] notice the panel shows in place of a bubble.
//!
//! - `use_skill {name}` is the activation stage: the `SKILL.md` body and the skill's file list.
//! - `read_skill_file {name, path}` reads a file under the skill, its path confined to the
//!   folder: text, JSON and YAML as text cut at [`ai_assets::TEXT_LIMIT`] characters, PNG, JPEG,
//!   GIF and WebP images as image parts (ai D19), counted toward the turn's
//!   [`ai_assets::IMAGE_LIMIT`] and held as `skill:<name>/<path>`, so a later turn loads them
//!   from the skill again; anything else as its path, media type and size.
//! - `install_skill_in_browser {name}` copies the skill's folder into the browser box at
//!   `/work/skills/<name>` through the exec service's `/run`, file by file in base64 chunks, so
//!   its scripts run with `browser_run`. It is offered with the browser tools (browser B6).
//!
//! The two read tools are in the agent's tool list like any and only read, so unattended runs
//! are offered them (agents G12). A skill's `allowed-tools` is shown and recorded but grants
//! nothing: the agent's own permissions decide its tools. Each call records an `ai.skill` entry
//! in the activity log (`crate::ai_activity`).

use ai::{Part, ToolSpec};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use schema::Agent;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use writer::{Skill, Writer};

use crate::ai_assets::{self, Images, Read};
use crate::ai_browser::{BrowserBox, BrowserOut};
use crate::ai_chat::tool;

/// The read tools, in the agent's tool list like any other.
pub const TOOLS: [&str; 2] = ["use_skill", "read_skill_file"];

/// The tool that copies a skill into the browser box, offered with the browser tools.
pub const INSTALL_TOOL: &str = "install_skill_in_browser";

/// Where a skill goes in the browser box.
pub const BOX_SKILLS: &str = "/work/skills";

/// The prefix of a skill image's key in the turn's images: `skill:<name>/<path>`.
pub const SKILL_IMAGE: &str = "skill:";

/// The notice kind of a `/update-skills` turn's user message.
pub const UPDATED_NOTICE: &str = "skills_updated";

/// The user message of a `/update-skills` turn, which the model reads.
pub const UPDATED_TEXT: &str =
    "The available skills changed; the list at the end of your instructions is current.";

/// The most bytes of a file one `/run` command carries, before base64.
const CHUNK: usize = 60 * 1024;

/// The image types sent as images.
const IMAGES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/// Whether `name` is one of the read tools.
pub fn is_skill_tool(name: &str) -> bool {
    TOOLS.contains(&name)
}

/// One skill of a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listed {
    pub name: String,
    pub description: String,
}

/// The skills a thread was offered, as its system text lists them (skills S6): kept in the
/// thread's transcript from its first turn until `/update-skills`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    #[serde(default)]
    pub skills: Vec<Listed>,
    /// The section as the system text ends with it; empty when no skill is offered.
    #[serde(default)]
    pub section: String,
}

impl Snapshot {
    /// The section that ends the system text, or `None` when no skill is offered.
    pub fn section(&self) -> Option<&str> {
        (!self.section.is_empty()).then_some(self.section.as_str())
    }
}

/// Whether `agent` uses skills at all: its skills list is not empty and its tools list, if any,
/// names `use_skill`.
pub fn uses_skills(agent: &Agent) -> bool {
    let p = &agent.permissions;
    p.skills.as_ref().is_none_or(|s| !s.is_empty()) && p.allows_tool("use_skill")
}

/// The valid skills `agent` is offered, by name.
pub fn offered(w: &Writer, agent: &Agent) -> Vec<Skill> {
    if !uses_skills(agent) {
        return Vec::new();
    }
    match w.valid_skills() {
        Ok(all) => all
            .into_iter()
            .filter(|s| agent.permissions.allows_skill(&s.name))
            .collect(),
        Err(e) => {
            eprintln!("the skills could not be listed: {e}");
            Vec::new()
        }
    }
}

/// The skills `agent` is offered now, as a thread keeps them. `browser` says the turn has the
/// browser box and so `install_skill_in_browser`.
pub fn snapshot(w: &Writer, agent: &Agent, browser: bool) -> Snapshot {
    let skills: Vec<Listed> = offered(w, agent)
        .into_iter()
        .map(|s| Listed {
            description: one_line(s.md.description.as_deref().unwrap_or_default()),
            name: s.name,
        })
        .collect();
    let section = if skills.is_empty() {
        String::new()
    } else {
        section(&skills, browser)
    };
    Snapshot { skills, section }
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn section(skills: &[Listed], browser: bool) -> String {
    let mut out = String::from(
        "\n## Available skills\n\nSkills are folders of instructions, and sometimes scripts and other files, \
         for particular tasks. When a task matches a skill's description, call use_skill with its \
         name before following it: that answers its instructions and its files. Read the files its \
         instructions point to with read_skill_file, by their paths in the skill. ",
    );
    if browser {
        out.push_str(
            "To run a skill's scripts, copy the skill into the browser box with \
             install_skill_in_browser, then run them there with browser_run from \
             /work/skills/<name>. ",
        );
    } else {
        out.push_str(
            "A skill's scripts can be read here but not run: say so when its instructions need \
             one run. ",
        );
    }
    out.push_str("A skill's allowed-tools grants nothing: use only the tools you have.\n\n");
    for s in skills {
        out.push_str(&format!("- `{}`: {}\n", s.name, s.description));
    }
    out
}

/// The read tools (skills S6).
pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        tool(
            "use_skill",
            "Activate a skill from the Available skills list at the end of your instructions, by name. Answers its instructions (the SKILL.md body) and the list of its files. Call it before following a skill; read the files its instructions point to with read_skill_file.",
            json!({ "name": { "type": "string", "description": "The skill's name, as the list gives it." } }),
        ),
        tool(
            "read_skill_file",
            "Read a file of a skill, by the skill's name and the file's path in the skill's folder (such as `references/forms.md`). Text, JSON and YAML come back as text, cut at 40000 characters; a PNG, JPEG, GIF or WebP image as the image itself, counted toward the 8 images per question; anything else as its size and type.",
            json!({
                "name": { "type": "string", "description": "The skill's name." },
                "path": { "type": "string", "description": "The file's path from the skill's folder, such as `scripts/fill.py`." },
            }),
        ),
    ]
}

/// `install_skill_in_browser`, offered with the browser tools.
pub fn install_spec() -> ToolSpec {
    tool(
        INSTALL_TOOL,
        "Copy a skill's folder into the browser box at /work/skills/<name>, replacing what is there, so its scripts can run with browser_run (for example `cd /work/skills/<name> && python3 scripts/run.py`). Files starting with #! are made executable.",
        json!({ "name": { "type": "string", "description": "The skill's name." } }),
    )
}

/// The skill `input` names, if `agent` is offered it.
fn skill_of(w: &Writer, agent: &Agent, input: &Value) -> Result<Skill, String> {
    let Some(name) = input["name"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Err("name must be a non-empty string".to_owned());
    };
    if !uses_skills(agent) || !agent.permissions.allows_skill(name) {
        return Err(format!("This agent is not offered the skill {name:?}."));
    }
    let skill = w.skill(name).map_err(|e| match e {
        writer::Error::Missing(_) => format!("there is no skill {name:?}"),
        e => e.to_string(),
    })?;
    if !skill.valid() {
        let errors: Vec<String> = skill
            .issues
            .iter()
            .filter(|i| !i.is_warning())
            .map(|i| format!("{}: {}", i.at, i.message))
            .collect();
        return Err(format!(
            "the skill {name:?} has errors, so it is offered to no agent: {}",
            errors.join("; ")
        ));
    }
    Ok(skill)
}

fn summary(skill: &str, kind: &str, extra: Value) -> Value {
    let mut s = json!({ "name": skill, "kind": kind });
    if let (Some(s), Some(e)) = (s.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            s.insert(k.clone(), v.clone());
        }
    }
    json!({ "notes": [], "skill": s })
}

fn read_error(message: impl Into<String>, skill: Option<&str>) -> Read {
    let message = message.into();
    let mut summary = json!({ "notes": [], "message": message });
    if let Some(name) = skill {
        summary["skill"] = json!({ "name": name, "kind": "error" });
    }
    Read {
        content: json!({ "error": message }).to_string(),
        is_error: true,
        parts: Vec::new(),
        summary,
    }
}

/// `use_skill {name}`: the instructions and the file list. `browser` says the turn may install
/// it in the browser box.
pub fn use_skill(w: &Writer, agent: &Agent, input: &Value, browser: bool) -> Read {
    let skill = match skill_of(w, agent, input) {
        Ok(s) => s,
        Err(e) => return read_error(e, input["name"].as_str()),
    };
    let md = &skill.md;
    let files: Vec<Value> = skill
        .files
        .iter()
        .map(|f| json!({ "path": f.path, "size": f.size, "media_type": f.media_type }))
        .collect();
    let scripts = if browser {
        format!(
            "To run its scripts, copy it into the browser box with install_skill_in_browser; it goes to {BOX_SKILLS}/{}.",
            skill.name
        )
    } else {
        "Its scripts can be read with read_skill_file but not run here.".to_owned()
    };
    let mut content = json!({
        "name": skill.name,
        "description": md.description,
        "instructions": ai_assets_cut(&md.body),
        "files": files,
        "note": format!(
            "Follow the instructions. Paths in them are relative to the skill's folder: read those files with read_skill_file. {scripts}"
        ),
    });
    // The skill's own script tools, offered from the next round (script-tools T4).
    let tools: Vec<String> = crate::script_tools::skill_tool_names(&skill)
        .iter()
        .filter_map(|t| crate::script_tools::skill_tool_name(&skill.name, t))
        .filter(|t| agent.permissions.allows_tool(t))
        .collect();
    if !tools.is_empty() {
        content["tools"] = json!(tools);
        content["tools_note"] = json!(
            "These are the skill's script tools (its tools/ folder). They are your tools from your next step on, for the rest of this conversation, when they are valid."
        );
    }
    if let Some(l) = &md.license {
        content["license"] = json!(l);
    }
    if let Some(c) = &md.compatibility {
        content["compatibility"] = json!(c);
    }
    if !md.metadata.is_empty() {
        content["metadata"] = json!(md.metadata);
    }
    let allowed = md.allowed_tools_list();
    if !allowed.is_empty() {
        content["allowed_tools"] = json!(allowed);
        content["allowed_tools_note"] =
            json!("allowed-tools is recorded but grants nothing: use only the tools you have.");
    }
    Read {
        content: content.to_string(),
        is_error: false,
        parts: Vec::new(),
        summary: summary(&skill.name, "use", json!({ "files": skill.files.len() })),
    }
}

fn ai_assets_cut(s: &str) -> String {
    cut(s, ai_assets::TEXT_LIMIT, false)
}

fn cut(s: &str, limit: usize, partial: bool) -> String {
    if !partial && s.chars().count() <= limit {
        return s.to_owned();
    }
    let kept: String = s.chars().take(limit).collect();
    format!("{kept}\n[truncated at {limit} characters; the whole text is longer]")
}

fn is_text(media: &str) -> bool {
    media.starts_with("text/")
        || matches!(
            media,
            "application/json"
                | "application/yaml"
                | "application/toml"
                | "application/xml"
                | "image/svg+xml"
        )
}

/// The key of image `path` of skill `name` in the turn's images.
pub fn image_key(name: &str, path: &str) -> String {
    format!("{SKILL_IMAGE}{name}/{path}")
}

/// Image `key` ([`image_key`]) read from its skill and prepared to send.
pub fn load_image(w: &Writer, key: &str, media: &str) -> Result<ai::ImageData, String> {
    let rest = key.strip_prefix(SKILL_IMAGE).unwrap_or(key);
    let Some((name, path)) = rest.split_once('/') else {
        return Err(format!("{key} names no skill file"));
    };
    let bytes = w
        .skill_file(name, path)
        .map_err(|_| format!("skill {name:?} no longer has {path}"))?;
    ai_assets::prepare(bytes, media)
}

/// `read_skill_file {name, path}`. `may_send_image` is false once the turn has sent its images.
pub fn read_skill_file(
    w: &Writer,
    agent: &Agent,
    input: &Value,
    may_send_image: bool,
    images: &Images,
) -> Read {
    let skill = match skill_of(w, agent, input) {
        Ok(s) => s,
        Err(e) => return read_error(e, input["name"].as_str()),
    };
    let Some(raw) = input["path"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return read_error("path must be a non-empty string", Some(&skill.name));
    };
    let path = match store::normalise_skill_path(raw) {
        Ok(p) => p,
        Err(e) => {
            return read_error(
                format!("{e}: give a path inside the skill's folder, as use_skill lists them"),
                Some(&skill.name),
            );
        }
    };
    let Some(info) = skill.file(&path).cloned() else {
        return read_error(
            format!(
                "skill {:?} has no file {path:?}: use_skill lists its files",
                skill.name
            ),
            Some(&skill.name),
        );
    };
    let bytes = match w.skill_file(&skill.name, &path) {
        Ok(b) => b,
        Err(e) => return read_error(e.to_string(), Some(&skill.name)),
    };
    let media = info.media_type;
    let mut meta = json!({
        "name": skill.name, "path": path, "media_type": media, "size": info.size,
    });
    let done = |content: Value, kind: &str, parts: Vec<Part>| Read {
        content: content.to_string(),
        is_error: false,
        parts,
        summary: summary(
            &skill.name,
            "file",
            json!({ "path": path, "media_type": media, "size": info.size, "read_as": kind }),
        ),
    };
    if IMAGES.contains(&media) {
        if !may_send_image {
            return read_error(
                format!(
                    "the limit of {} images per question is reached: this image was not sent. \
                     Answer from what you have, or ask the user to ask again.",
                    ai_assets::IMAGE_LIMIT
                ),
                Some(&skill.name),
            );
        }
        let key = image_key(&skill.name, &path);
        let prepared = ai_assets::prepare(bytes, media);
        images
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.clone(), prepared.clone());
        return match prepared {
            Ok(sent) => {
                meta["sent_as"] = json!(sent.media_type);
                meta["note"] = json!("The image follows.");
                done(meta, "image", vec![Part::image(key, media)])
            }
            Err(why) => {
                meta["note"] = json!(format!("The image cannot be sent: {why}."));
                done(meta, "metadata", Vec::new())
            }
        };
    }
    if info.text || is_text(media) {
        // Four bytes a character at most, and one more to tell a longer file.
        let want = (ai_assets::TEXT_LIMIT * 4 + 1).min(bytes.len());
        let partial = want < bytes.len();
        let mut text = String::from_utf8_lossy(&bytes[..want]).into_owned();
        if partial {
            // The cut may have split the last character.
            text.pop();
        }
        meta["text"] = json!(cut(&text, ai_assets::TEXT_LIMIT, partial));
        return done(meta, "text", Vec::new());
    }
    meta["note"] = json!(
        "This file is not text or an image the model can see. Copy the skill into the browser box to use it there, when you have it."
    );
    done(meta, "metadata", Vec::new())
}

/// `s` quoted for bash.
fn sh(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The commands that copy `files` (path and bytes) into `dir`, each holding at most about
/// [`CHUNK`] bytes of a file in base64.
fn install_commands(dir: &str, files: &[(String, Vec<u8>)]) -> Vec<String> {
    let mut commands = vec![format!("rm -rf {0} && mkdir -p {0}", sh(dir))];
    let mut current = String::new();
    let mut size = 0usize;
    let mut push = |part: String, len: usize, current: &mut String, size: &mut usize| {
        if *size + len > CHUNK && !current.is_empty() {
            commands.push(std::mem::take(current));
            *size = 0;
        }
        if !current.is_empty() {
            current.push_str(" && ");
        }
        current.push_str(&part);
        *size += len;
    };
    for (path, bytes) in files {
        let target = format!("{dir}/{path}");
        let parent = target.rsplit_once('/').map_or(dir, |(p, _)| p).to_owned();
        let mut first = true;
        let chunks: Vec<&[u8]> = if bytes.is_empty() {
            vec![&[][..]]
        } else {
            bytes.chunks(CHUNK).collect()
        };
        for chunk in chunks {
            let redirect = if first { ">" } else { ">>" };
            let part = if first {
                format!(
                    "mkdir -p {} && printf %s {} | base64 -d {redirect} {}",
                    sh(&parent),
                    sh(&STANDARD.encode(chunk)),
                    sh(&target)
                )
            } else {
                format!(
                    "printf %s {} | base64 -d {redirect} {}",
                    sh(&STANDARD.encode(chunk)),
                    sh(&target)
                )
            };
            first = false;
            push(part, chunk.len(), &mut current, &mut size);
        }
        if bytes.starts_with(b"#!") {
            push(
                format!("chmod +x {}", sh(&target)),
                0,
                &mut current,
                &mut size,
            );
        }
    }
    if !current.is_empty() {
        commands.push(current);
    }
    commands
}

/// `install_skill_in_browser {name}`: the skill's files copied into the box, one `/run` command
/// after another, stopping at the first that fails.
pub async fn install(
    w: std::sync::Arc<Writer>,
    b: &BrowserBox,
    agent: &Agent,
    input: &Value,
) -> BrowserOut {
    let name = input["name"].as_str().unwrap_or_default().trim().to_owned();
    let fail = |message: String| {
        let mut out = BrowserOut::error(message);
        if !name.is_empty() {
            out.summary["skill"] = json!({ "name": name, "kind": "install" });
        }
        out
    };
    let agent = agent.clone();
    let input = input.clone();
    let read = tokio::task::spawn_blocking(move || {
        let skill = skill_of(&w, &agent, &input)?;
        let mut files = Vec::with_capacity(skill.files.len());
        for f in &skill.files {
            let bytes = w
                .skill_file(&skill.name, &f.path)
                .map_err(|e| e.to_string())?;
            files.push((f.path.clone(), bytes));
        }
        Ok::<_, String>((skill.name, files))
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    let (skill, files) = match read {
        Ok(r) => r,
        Err(e) => return fail(e),
    };
    let dir = format!("{BOX_SKILLS}/{skill}");
    let commands = install_commands(&dir, &files);
    let bytes: usize = files.iter().map(|(_, b)| b.len()).sum();
    for (i, command) in commands.iter().enumerate() {
        let ran = crate::ai_browser::run_quiet(b, command, 120).await;
        if let Err(why) = ran {
            let mut out = fail(format!(
                "copying the skill into the browser box stopped at step {} of {}: {why}",
                i + 1,
                commands.len()
            ));
            out.trail = Some(format!(
                "browser: install skill={skill} files={} failed={why:?}",
                files.len()
            ));
            return out;
        }
    }
    let content = json!({
        "installed": dir,
        "files": files.iter().map(|(p, _)| p).collect::<Vec<_>>(),
        "bytes": bytes,
        "note": format!("Run its scripts with browser_run, from {dir}."),
    });
    BrowserOut {
        content: content.to_string(),
        is_error: false,
        parts: Vec::new(),
        summary: summary(
            &skill,
            "install",
            json!({ "files": files.len(), "bytes": bytes, "dir": dir }),
        ),
        event: None,
        trail: Some(format!(
            "browser: install skill={skill} files={} bytes={bytes} runs={}",
            files.len(),
            commands.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_section_lists_each_skill_and_says_how_to_run_scripts() {
        let skills = vec![Listed {
            name: "pdf-forms".into(),
            description: "Fill PDF forms.".into(),
        }];
        let s = section(&skills, true);
        assert!(s.starts_with("\n## Available skills\n"), "{s}");
        assert!(s.contains("- `pdf-forms`: Fill PDF forms.\n"));
        assert!(s.contains("install_skill_in_browser"));
        let s = section(&skills, false);
        assert!(s.contains("can be read here but not run"), "{s}");
        assert!(!s.contains("install_skill_in_browser"));
    }

    #[test]
    fn install_commands_chunk_files_and_quote_paths() {
        let big = vec![b'x'; CHUNK * 2 + 10];
        let files = vec![
            ("SKILL.md".to_owned(), b"---\nname: a\n---\n".to_vec()),
            (
                "scripts/it's.sh".to_owned(),
                b"#!/bin/sh\necho hi\n".to_vec(),
            ),
            ("assets/big.bin".to_owned(), big.clone()),
            ("empty.txt".to_owned(), Vec::new()),
        ];
        let c = install_commands("/work/skills/a", &files);
        assert_eq!(c[0], "rm -rf '/work/skills/a' && mkdir -p '/work/skills/a'");
        let all = c.join("\n");
        assert!(
            all.contains("chmod +x '/work/skills/a/scripts/it'\\''s.sh'"),
            "{all}"
        );
        assert!(all.contains("> '/work/skills/a/empty.txt'"));
        assert_eq!(all.matches(">> '/work/skills/a/assets/big.bin'").count(), 2);
        // No command carries much more than a chunk, so each stays well under bash's 128 KiB.
        assert!(
            c.iter().all(|c| c.len() < CHUNK * 2),
            "{:?}",
            c.iter().map(String::len).collect::<Vec<_>>()
        );
        // The chunks decode back to the file.
        let mut got = Vec::new();
        for part in c
            .iter()
            .flat_map(|c| c.split(" && "))
            .filter(|p| p.contains("big.bin") && p.contains("base64"))
        {
            let b64 = part.split('\'').nth(1).unwrap();
            got.extend(STANDARD.decode(b64).unwrap());
        }
        assert_eq!(got, big);
    }
}
