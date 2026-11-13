//! Canonical server-managed secret names and the generic write boundary.
pub const AI_ACCOUNTS: &str = "ai_accounts";
pub const AI_SETTINGS: &str = "ai_settings";
pub const AI_KEY_PREFIX: &str = "ai_account_key_";
pub const AI_CHATGPT_PREFIX: &str = "ai_account_chatgpt_";
pub const AI_USAGE_PREFIX: &str = "ai_account_usage_";
pub const GOOGLE_SETTINGS: &str = "google_settings";
pub const GOOGLE_CLIENT_ID: &str = "google_client_id";
pub const GOOGLE_CLIENT_SECRET: &str = "google_client_secret";
pub const GOOGLE_REFRESH_TOKEN: &str = "google_refresh_token";
pub const GOOGLE_ACCOUNT: &str = "google_account";
pub const GOOGLE_GMAIL_STATE: &str = "google_gmail_state";
pub const GOOGLE_CALENDAR_STATE: &str = "google_calendar_state";
pub const GOOGLE_GMAIL_TRASHED: &str = "google_gmail_trashed";
pub const GITSYNC_SETTINGS: &str = "gitsync_settings";
pub const GITSYNC_TOKEN: &str = "gitsync_token";
pub const GITSYNC_SSH_KEY: &str = "gitsync_ssh_key";
pub const GITSYNC_KNOWN_HOSTS: &str = "gitsync_known_hosts";
pub const GITSYNC_KEY: &str = "gitsync_key";
pub const GITSYNC_STATE: &str = "gitsync_state";
pub const GITSYNC_FRESHNESS_PUBLISH_PREFIX: &str = "gitsync_freshness_publish_";
pub const GITSYNC_FRESHNESS_RECEIVE_PREFIX: &str = "gitsync_freshness_receive_";
pub const BRAVE_API_KEY: &str = "brave_api_key";
pub const BRAVE_SETTINGS: &str = "brave_settings";

/// Exact managed names and dynamic families. Not a blanket vendor-prefix reservation.
pub fn reserved_secret(name: &str) -> bool {
    [
        AI_ACCOUNTS,
        AI_SETTINGS,
        GOOGLE_SETTINGS,
        GOOGLE_CLIENT_ID,
        GOOGLE_CLIENT_SECRET,
        GOOGLE_REFRESH_TOKEN,
        GOOGLE_ACCOUNT,
        GOOGLE_GMAIL_STATE,
        GOOGLE_CALENDAR_STATE,
        GOOGLE_GMAIL_TRASHED,
        GITSYNC_SETTINGS,
        GITSYNC_TOKEN,
        GITSYNC_SSH_KEY,
        GITSYNC_KNOWN_HOSTS,
        GITSYNC_KEY,
        GITSYNC_STATE,
        BRAVE_API_KEY,
    ]
    .contains(&name)
        || [
            AI_KEY_PREFIX,
            AI_CHATGPT_PREFIX,
            AI_USAGE_PREFIX,
            GITSYNC_FRESHNESS_PUBLISH_PREFIX,
            GITSYNC_FRESHNESS_RECEIVE_PREFIX,
        ]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// Only these records are fixture seams. The caller must separately require a test build,
/// explicit startup opt-in and an owner session. Tokens never get this exception.
pub fn test_seam_secret(name: &str) -> bool {
    [
        AI_SETTINGS,
        GOOGLE_SETTINGS,
        GOOGLE_CLIENT_ID,
        GOOGLE_CLIENT_SECRET,
        GOOGLE_REFRESH_TOKEN,
        GOOGLE_ACCOUNT,
    ]
    .contains(&name)
        || name.starts_with(AI_CHATGPT_PREFIX)
}
