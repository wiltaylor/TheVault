//! Durable token counts only. No prompts, content, credentials or prices.
use crate::{Result, State};
use rusqlite::params;

pub const AI_USAGE_RETENTION_MS: i64 = 90 * 24 * 60 * 60 * 1000;
pub(crate) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS ai_usage (account TEXT NOT NULL, model TEXT NOT NULL, purpose TEXT NOT NULL, input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL, at_ms INTEGER NOT NULL); CREATE INDEX IF NOT EXISTS ai_usage_at ON ai_usage(at_ms);";

#[derive(Debug, serde::Serialize)]
pub struct AiUsageDaily {
    pub day: String,
    pub account: String,
    pub model: String,
    pub purpose: String,
    pub calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

impl State {
    pub fn record_ai_usage(&self, account: &str, model: &str, purpose: &str, input: u64, output: u64) -> Result<()> {
        self.conn().execute("INSERT INTO ai_usage VALUES (?1,?2,?3,?4,?5,?6)", params![account, model, purpose, input.min(i64::MAX as u64) as i64, output.min(i64::MAX as u64) as i64, self.now()])?;
        Ok(())
    }

    /// UTC daily totals, separated by account, model and purpose.
    pub fn ai_usage_daily(&self) -> Result<Vec<AiUsageDaily>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT strftime('%Y-%m-%d',at_ms/1000,'unixepoch'),account,model,purpose,count(*),sum(input_tokens),sum(output_tokens) FROM ai_usage GROUP BY 1,2,3,4 ORDER BY 1 DESC,2,3,4")?;
        Ok(stmt.query_map([], |r| Ok(AiUsageDaily { day:r.get(0)?, account:r.get(1)?, model:r.get(2)?, purpose:r.get(3)?, calls:r.get(4)?, input_tokens:r.get(5)?, output_tokens:r.get(6)? }))?.collect::<std::result::Result<Vec<_>,_>>()?)
    }

    pub(crate) fn prune_ai_usage(&self) -> Result<usize> {
        Ok(self.conn().execute("DELETE FROM ai_usage WHERE at_ms < ?1", [self.now().saturating_sub(AI_USAGE_RETENTION_MS)])?)
    }
}
