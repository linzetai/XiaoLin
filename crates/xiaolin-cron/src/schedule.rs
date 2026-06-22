use std::str::FromStr;

use chrono::Utc;
use cron::Schedule;

/// Compute the next scheduled run time after `now`.
///
/// Cron expressions are interpreted in the **system's local timezone** so that
/// `"0 9 * * *"` means "9 AM on the machine running XiaoLin", regardless of the
/// UTC offset of the server.  The returned RFC3339 string is **normalized to UTC**
/// (offset `+00:00`) so all `next_run` values in the database are directly comparable
/// with `Utc::now()` via SQLite text collation.
pub fn compute_next_run(schedule_str: &str) -> Option<String> {
    let schedule = Schedule::from_str(schedule_str).ok()?;
    let next_local = schedule.upcoming(chrono::Local).next()?;
    Some(next_local.with_timezone(&Utc).to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cron_schedule() {
        let next = compute_next_run("0 */5 * * * *");
        assert!(next.is_some(), "should parse a 6-field cron expression");
    }

    #[test]
    fn invalid_schedule_returns_none() {
        assert!(compute_next_run("not a cron").is_none());
    }
}
