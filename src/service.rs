//! The service: reports the inventory on its own, at the interval opensecmon asks for.
//!
//! Each round reads the agent's files again, so an enrollment made while the service runs (from the enrollment
//! dialog) is picked up within a minute, and a revoked computer stays quiet until it is enrolled again.

use std::hash::{BuildHasher, Hasher};
use std::time::Duration;

use crate::agent::{self, Agent, Error};
use crate::api::ApiError;
use crate::inventory;

/// How often the files are looked at while the computer is not enrolled or was revoked (no network).
const IDLE_S: u64 = 60;
/// Wait after the start before an overdue report, so the network and the session are up after a boot.
const START_DELAY_S: u64 = 60;
/// Waits after failed rounds in a row: 5, 15, then 60 min.
const BACKOFF_S: [u64; 3] = [300, 900, 3600];
/// The interval if opensecmon has not named one yet (its value for opensecagent).
const DEFAULT_INTERVAL_S: u64 = 900;
/// Bounds for the intervals and waits opensecmon names.
const INTERVAL_MIN_S: u64 = 60;
const INTERVAL_MAX_S: u64 = 24 * 3600;

/// What a round of the service came to.
#[derive(Debug)]
enum Round {
    /// Not enrolled, or revoked: look again later.
    Idle,
    /// The next report is not due yet.
    NotDue(u64),
    /// Reported; the next report in this many seconds.
    Reported(u64),
    /// opensecmon asked to wait this long.
    Wait(u64),
    /// Something failed; the backoff applies.
    Failed,
}

/// Runs until the service manager asks it to stop.
pub async fn run(agent: Agent) -> Result<(), Error> {
    println!("opensecagent {} started", crate::api::AGENT_VERSION);
    let mut failures = 0usize;
    let mut first = true;
    let stop = stop_signal();
    tokio::pin!(stop);
    loop {
        let round = if first {
            first = false;
            match (agent.is_enrolled(), due_in(&agent)) {
                (false, _) => Round::Idle,
                (true, Some(s @ 1..)) => Round::NotDue(s),
                (true, _) => Round::NotDue(START_DELAY_S),
            }
        } else {
            round(&agent).await
        };
        failures = if matches!(round, Round::Failed) {
            failures + 1
        } else {
            0
        };
        let wait = wait_after(&round, failures, random());
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(wait)) => {}
            () = &mut stop => {
                println!("opensecagent stopped");
                return Ok(());
            }
        }
    }
}

/// Seconds until the next report is due; `None` if the computer is not enrolled, revoked or never reported.
fn due_in(agent: &Agent) -> Option<u64> {
    let state = agent.state();
    if state.revoked {
        return None;
    }
    let last = state.last_report?;
    let interval = bounded(state.next_report_s.unwrap_or(DEFAULT_INTERVAL_S));
    Some((last + interval).saturating_sub(agent::now()).min(interval))
}

async fn round(agent: &Agent) -> Round {
    if !agent.is_enrolled() {
        return Round::Idle;
    }
    if let Some(s @ 1..) = due_in(agent) {
        return Round::NotDue(s);
    }
    let inventory = match inventory::collect() {
        Ok(i) => i,
        Err(e) => {
            let e = Error::Inventory(e);
            println!("the inventory could not be read, the report waits: {e}");
            agent.note_error(&e);
            return Round::Failed;
        }
    };
    match agent.report(&inventory, false).await {
        Ok(r) => {
            if r.short {
                println!("inventory unchanged (sent its hash)");
            } else {
                println!("inventory sent ({} KB)", r.size.div_ceil(1024));
            }
            for w in &r.warnings {
                println!("opensecmon skipped {} entries ({})", w.count, w.code);
            }
            Round::Reported(r.next_report_s)
        }
        Err(Error::NotEnrolled) => Round::Idle,
        Err(e @ Error::Revoked) => {
            println!("{e}");
            Round::Idle
        }
        Err(Error::Wait(s)) => {
            println!("opensecmon asks to wait {s} s");
            Round::Wait(s)
        }
        Err(e @ Error::Api(ApiError::Http(_))) => {
            println!("opensecmon could not be reached: {e}");
            Round::Failed
        }
        Err(e) => {
            println!("the report did not work: {e}");
            Round::Failed
        }
    }
}

/// The wait after a round, with up to 10 % jitter on the report interval so that many computers do not report
/// at the same moment.
fn wait_after(round: &Round, failures: usize, random: u64) -> u64 {
    match *round {
        Round::Idle => IDLE_S,
        Round::NotDue(s) => s.clamp(1, INTERVAL_MAX_S),
        Round::Reported(s) => {
            let s = bounded(s);
            s + random % (s / 10 + 1)
        }
        Round::Wait(s) => s.clamp(1, INTERVAL_MAX_S),
        Round::Failed => BACKOFF_S[failures.clamp(1, BACKOFF_S.len()) - 1],
    }
}

fn bounded(interval: u64) -> u64 {
    interval.clamp(INTERVAL_MIN_S, INTERVAL_MAX_S)
}

/// A random number for the jitter, from the random keys of the standard library.
fn random() -> u64 {
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(agent::now());
    h.finish()
}

/// Finishes when the service manager stops the service (SIGTERM, or Ctrl+C in a terminal).
async fn stop_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = term.recv() => {}
                    _ = tokio::signal::ctrl_c() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{State, Store};

    #[test]
    fn waits_follow_the_round() {
        assert_eq!(wait_after(&Round::Idle, 0, 7), 60);
        assert_eq!(wait_after(&Round::NotDue(0), 0, 7), 1);
        assert_eq!(wait_after(&Round::NotDue(400), 0, 7), 400);
        assert_eq!(wait_after(&Round::Wait(120), 0, 7), 120);
        assert_eq!(wait_after(&Round::Wait(10_000_000), 0, 7), 86_400);
        // The interval plus at most 10 %.
        assert_eq!(wait_after(&Round::Reported(900), 0, 0), 900);
        assert_eq!(wait_after(&Round::Reported(900), 0, 90), 990);
        assert_eq!(wait_after(&Round::Reported(900), 0, 91), 900);
        for r in [1u64, 12_345, u64::MAX] {
            let w = wait_after(&Round::Reported(900), 0, r);
            assert!((900..=990).contains(&w), "{w}");
        }
        // Silly intervals from the server are bounded.
        assert_eq!(wait_after(&Round::Reported(0), 0, 0), 60);
        assert_eq!(wait_after(&Round::Reported(u64::MAX), 0, 0), 86_400);
    }

    #[test]
    fn failures_back_off() {
        let waits: Vec<u64> = (1..=5).map(|n| wait_after(&Round::Failed, n, 0)).collect();
        assert_eq!(waits, [300, 900, 3600, 3600, 3600]);
    }

    #[test]
    fn the_next_report_is_due_after_the_interval() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let agent = Agent::new(Store::new(dir.path().to_path_buf()));
        assert_eq!(due_in(&agent), None);
        let now = agent::now();
        let mut state = State {
            last_report: Some(now - 100),
            next_report_s: Some(900),
            ..State::default()
        };
        store.save_state(&state).unwrap();
        let due = due_in(&agent).unwrap();
        assert!((799..=800).contains(&due), "{due}");
        state.last_report = Some(now - 5000);
        store.save_state(&state).unwrap();
        assert_eq!(due_in(&agent), Some(0));
        // A clock set back does not stop the reports for longer than one interval.
        state.last_report = Some(now + 1_000_000);
        store.save_state(&state).unwrap();
        assert_eq!(due_in(&agent), Some(900));
        state.revoked = true;
        store.save_state(&state).unwrap();
        assert_eq!(due_in(&agent), None);
    }
}
