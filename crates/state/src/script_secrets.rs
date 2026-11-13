//! Names owned by the server, not by Settings > Secrets scripts.

/// Google credentials are also the declared grants of the shipped sync and outbox scripts.
/// They remain available to scripts, but may not be granted by an agent proposal.
pub fn internal_script_secret(name: &str) -> bool {
    (crate::reserved_secret(name) && !name.starts_with("google_"))
        || name == crate::BRAVE_SETTINGS
        || name.starts_with("ai_account_")
        || name.starts_with("gitsync_")
}

/// Refuse a server-owned name in a task or tool proposed by an agent.
pub fn proposed_secrets_refused(names: &[String]) -> Option<String> {
    names.iter().find(|name| crate::reserved_secret(name) || internal_script_secret(name) || name.starts_with("google_")).map(|name| {
        format!("secret {name:?} is reserved for the server and cannot be granted by an agent proposal")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_names_are_reserved_but_user_names_and_shipped_google_grants_work() {
        for name in [
            "ai_settings",
            "ai_accounts",
            "ai_account_key_id",
            "ai_account_chatgpt_id",
            "ai_account_usage_id",
            "gitsync_token",
            "gitsync_settings",
            "gitsync_ssh_key",
            "gitsync_key",
            "gitsync_state",
            "gitsync_known_hosts",
            "brave_api_key",
            "brave_settings",
        ] {
            assert!(internal_script_secret(name), "{name}");
            assert!(proposed_secrets_refused(&[name.into()]).is_some(), "{name}");
        }
        for name in [
            "google_client_id",
            "google_client_secret",
            "google_refresh_token",
            "google_settings",
            "google_account",
            "google_gmail_state",
            "google_calendar_state",
            "google_gmail_trashed",
        ] {
            assert!(!internal_script_secret(name), "{name}");
            assert!(proposed_secrets_refused(&[name.into()]).is_some(), "{name}");
        }
        for name in ["weather_key", "hook_token", "state", "my_google_token"] {
            assert!(!internal_script_secret(name), "{name}");
            assert!(proposed_secrets_refused(&[name.into()]).is_none(), "{name}");
        }
    }
}
