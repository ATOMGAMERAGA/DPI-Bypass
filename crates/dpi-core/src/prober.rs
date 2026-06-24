//! The auto-solver ("strategy finder").
//!
//! Given a target domain set and an [`Engine`], the prober first checks whether
//! the target is already reachable (no bypass needed). If it is blocked, it
//! walks the curated candidate list: apply → re-check → revert, stopping at the
//! first strategy that makes the target fully reachable (text, and voice when
//! requested).
//!
//! The reachability check is injected as a closure so the prober can be unit
//! tested without real network or root, and so the GUI can route checks through
//! whatever transport it likes.

use crate::engine::{DomainSet, Engine};
use crate::network::DomainCheck;
use crate::strategy::{candidate_strategies, Strategy};
use crate::Result;
use serde::{Deserialize, Serialize};

/// What the prober concluded.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ProbeOutcome {
    /// Target was reachable without any intervention.
    AlreadyOpen { check: DomainCheck },
    /// A working strategy was found.
    Found {
        strategy: Strategy,
        check: DomainCheck,
    },
    /// No candidate worked.
    NotFound,
}

/// Run the solver.
///
/// * `engine` — applies/reverts candidate strategies.
/// * `domains` — target set; its first entry is the probe target.
/// * `with_voice` — also require the voice (UDP/QUIC) probe to pass.
/// * `check` — reachability probe: `(domain, with_voice) -> DomainCheck`.
pub fn find_strategy<E, F>(
    engine: &mut E,
    domains: &DomainSet,
    with_voice: bool,
    mut check: F,
) -> Result<ProbeOutcome>
where
    E: Engine,
    F: FnMut(&str, bool) -> DomainCheck,
{
    let target = domains
        .domains
        .first()
        .cloned()
        .unwrap_or_else(|| "discord.com".to_string());

    // 1. Is it already open? Then do nothing.
    let baseline = check(&target, with_voice);
    if baseline.is_fully_open() {
        return Ok(ProbeOutcome::AlreadyOpen { check: baseline });
    }

    // 2. Try each candidate; revert between attempts so failures never linger.
    for strategy in candidate_strategies() {
        engine.apply(&strategy, domains)?;
        let result = check(&target, with_voice);
        engine.revert()?;

        if result.is_fully_open() {
            return Ok(ProbeOutcome::Found {
                strategy,
                check: result,
            });
        }
    }

    Ok(ProbeOutcome::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::NoopEngine;
    use crate::network::Reachability;

    fn open() -> DomainCheck {
        DomainCheck {
            text: Reachability::Reachable,
            voice: Some(Reachability::Reachable),
        }
    }
    fn blocked() -> DomainCheck {
        DomainCheck {
            text: Reachability::TlsReset,
            voice: Some(Reachability::Timeout),
        }
    }

    #[test]
    fn already_open_does_nothing() {
        let mut eng = NoopEngine::default();
        let out = find_strategy(&mut eng, &DomainSet::discord(), true, |_, _| open()).unwrap();
        assert!(matches!(out, ProbeOutcome::AlreadyOpen { .. }));
        assert!(eng.applied.is_empty());
    }

    #[test]
    fn finds_first_working_candidate() {
        let mut eng = NoopEngine::default();
        let mut calls = 0;
        // Blocked at baseline, then the *second* applied strategy works.
        let out = find_strategy(&mut eng, &DomainSet::discord(), true, |_, _| {
            calls += 1;
            // call 1 = baseline (blocked), call 2 = first candidate (blocked),
            // call 3 = second candidate (open)
            if calls >= 3 {
                open()
            } else {
                blocked()
            }
        })
        .unwrap();
        match out {
            ProbeOutcome::Found { .. } => {}
            other => panic!("expected Found, got {other:?}"),
        }
        // Engine must be reverted after probing, never left active.
        assert!(!eng.is_active());
    }

    #[test]
    fn no_candidate_works() {
        let mut eng = NoopEngine::default();
        let out = find_strategy(&mut eng, &DomainSet::discord(), true, |_, _| blocked()).unwrap();
        assert!(matches!(out, ProbeOutcome::NotFound));
        assert!(!eng.is_active());
    }
}
