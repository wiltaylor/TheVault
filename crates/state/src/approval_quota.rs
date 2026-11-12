//! Exact UTC target-mutation reservations, not coalesced activity counts.
//! A pending reservation survives a crash and is never automatically retried or expired.
use crate::{Result, State};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS approval_quota (
    id TEXT PRIMARY KEY, agent TEXT NOT NULL, rule TEXT NOT NULL,
    day INTEGER NOT NULL, destructive INTEGER NOT NULL,
    outcome TEXT NOT NULL CHECK(outcome IN ('reserved','succeeded','no_write'))
);
CREATE INDEX IF NOT EXISTS approval_quota_day ON approval_quota(day,agent,rule);
";

#[derive(Clone, Copy, Debug)]
pub struct ApprovalBudget {
    pub rule_daily: i64,
    /// Both required for destructive mutations. None or zero grants no allowance.
    pub agent_destructive_daily: Option<i64>,
    pub vault_destructive_daily: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalReservation {
    Reserved,
    Capped,
    /// Includes in-flight, interrupted and succeeded attempts. Never retry them.
    AlreadyAttempted,
}

impl State {
    /// Atomically reserves one target against all applicable budgets, even across connections.
    pub fn reserve_approval(&self, id: &str, agent: &str, rule: &str, destructive: bool, budget: ApprovalBudget) -> Result<ApprovalReservation> {
        let day = self.now().div_euclid(86_400_000);
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous: Option<String> = tx.query_row("SELECT outcome FROM approval_quota WHERE id=?1", [id], |r| r.get(0)).optional()?;
        if previous.as_deref().is_some_and(|s| s != "no_write") {
            return Ok(ApprovalReservation::AlreadyAttempted);
        }
        let rule_count: i64 = tx.query_row("SELECT count(*) FROM approval_quota WHERE day=?1 AND agent=?2 AND rule=?3 AND outcome!='no_write'", params![day, agent, rule], |r| r.get(0))?;
        if budget.rule_daily <= rule_count {
            return Ok(ApprovalReservation::Capped);
        }
        if destructive {
            let (agent_count, vault_count): (i64, i64) = tx.query_row("SELECT coalesce(sum(agent=?2),0),count(*) FROM approval_quota WHERE day=?1 AND destructive=1 AND outcome!='no_write'", params![day, agent], |r| Ok((r.get(0)?, r.get(1)?)))?;
            if budget.agent_destructive_daily.unwrap_or(0) <= agent_count || budget.vault_destructive_daily.unwrap_or(0) <= vault_count {
                return Ok(ApprovalReservation::Capped);
            }
        }
        tx.execute("INSERT INTO approval_quota VALUES (?1,?2,?3,?4,?5,'reserved') ON CONFLICT(id) DO UPDATE SET agent=excluded.agent,rule=excluded.rule,day=excluded.day,destructive=excluded.destructive,outcome='reserved'", params![id, agent, rule, day, destructive])?;
        tx.commit()?;
        Ok(ApprovalReservation::Reserved)
    }

    /// Only a proven no-write failure releases capacity. An uncertain failure stays reserved.
    pub fn finish_approval(&self, id: &str, succeeded: bool, proven_no_write: bool) -> Result<()> {
        let outcome = if succeeded { "succeeded" } else if proven_no_write { "no_write" } else { "reserved" };
        self.conn().execute("UPDATE approval_quota SET outcome=?2 WHERE id=?1 AND outcome='reserved'", params![id, outcome])?;
        Ok(())
    }
}
