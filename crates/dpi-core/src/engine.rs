//! Engine abstraction.
//!
//! An [`Engine`] knows how to make a [`Strategy`] take effect for a set of
//! domains, and how to fully revert. The concrete Linux engine (nftables +
//! nfqws) runs privileged and lives in `dpi-helper`; here we keep the trait plus
//! a no-op implementation used by tests and by the prober when running in a
//! dry-run mode.

use crate::strategy::Strategy;
use crate::Result;

/// The set of hostnames a strategy should be scoped to. The engine resolves
/// these to IPs and builds packet filters; an empty set means "system-wide".
#[derive(Debug, Clone, Default)]
pub struct DomainSet {
    pub domains: Vec<String>,
}

impl DomainSet {
    pub fn new(domains: Vec<String>) -> Self {
        Self { domains }
    }

    /// The default Discord domain set (text, API, gateway, CDN, voice/media).
    pub fn discord() -> Self {
        Self::new(
            [
                "discord.com",
                "discordapp.com",
                "discord.gg",
                "discord.media",
                "cdn.discordapp.com",
                "gateway.discord.gg",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        )
    }
}

/// Applies and reverts DPI-bypass strategies.
pub trait Engine {
    /// Make `strategy` active for `domains`. Must record enough state to fully
    /// revert later (kill-switch / rollback requirement).
    fn apply(&mut self, strategy: &Strategy, domains: &DomainSet) -> Result<()>;

    /// Undo everything `apply` did, returning the system to a clean state.
    fn revert(&mut self) -> Result<()>;

    /// Whether a strategy is currently applied.
    fn is_active(&self) -> bool;
}

/// An engine that records calls but touches nothing. Used in tests and as the
/// prober's backend when no privileged helper is available.
#[derive(Debug, Default)]
pub struct NoopEngine {
    pub active: bool,
    pub applied: Vec<Strategy>,
}

impl Engine for NoopEngine {
    fn apply(&mut self, strategy: &Strategy, _domains: &DomainSet) -> Result<()> {
        self.applied.push(strategy.clone());
        self.active = true;
        Ok(())
    }

    fn revert(&mut self) -> Result<()> {
        self.active = false;
        Ok(())
    }

    fn is_active(&self) -> bool {
        self.active
    }
}
