//! # tpt-control-priority
//!
//! Priority arbitration for competing control sources:
//! **Safety > Manual Override > Auto-Optimization > Schedule**.
//!
//! Resolves a set of competing setpoints for an actuator into a single winning command plus an
//! explicit reason. Tie-break rules are deterministic and documented below.
//!
//! `no_std` + `alloc`: pure logic.

#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

use alloc::vec::Vec;
use tpt_control_action::{ActuatorId, CommandEnvelope};
use tpt_control_audit::ReasonKind;

/// Control-source priority tier, ordered highest → lowest by `Ord`.
///
/// `Schedule < AutoOptimization < ManualOverride < Safety`, so the derived `Ord` lets us pick the
/// winner with a single comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PriorityTier {
    /// Least priority — scheduled/recipe setpoints.
    Schedule,
    /// Automatic optimization (the Brain).
    AutoOptimization,
    /// Human manual override.
    ManualOverride,
    /// Highest priority — safety systems.
    Safety,
}

impl PriorityTier {
    /// Numeric rank (higher = more authoritative).
    pub fn rank(self) -> u8 {
        match self {
            PriorityTier::Schedule => 1,
            PriorityTier::AutoOptimization => 2,
            PriorityTier::ManualOverride => 3,
            PriorityTier::Safety => 4,
        }
    }
}

impl core::fmt::Display for PriorityTier {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            PriorityTier::Schedule => "schedule",
            PriorityTier::AutoOptimization => "auto",
            PriorityTier::ManualOverride => "manual",
            PriorityTier::Safety => "safety",
        };
        f.write_str(s)
    }
}

/// A control source identified by id and priority tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ControlSource {
    pub id: u32,
    pub tier: PriorityTier,
}

/// A command tagged with the source that proposed it.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PrioritizedCommand {
    pub source: ControlSource,
    pub envelope: CommandEnvelope,
}

/// Result of arbitration: the winning command and why it won.
#[derive(Clone, Debug, PartialEq)]
pub struct ArbitrationOutcome {
    pub winner: PrioritizedCommand,
    /// Why the winner prevailed.
    pub reason: ReasonKind,
    /// Source ids that were overridden (lost the arbitration).
    pub overridden_sources: Vec<u32>,
}

/// The arbitration engine.
#[derive(Clone, Copy, Debug, Default)]
pub struct ArbitrationEngine;

impl ArbitrationEngine {
    /// Arbitrate a set of competing commands for (implicitly) one actuator.
    ///
    /// **Rules:**
    /// 1. Highest [`PriorityTier`] wins.
    /// 2. On a tier tie, the most recent command (highest `timestamp_ms`) wins.
    /// 3. On a full tie (same tier *and* timestamp), the lowest `source.id` wins (deterministic).
    ///
    /// Returns `None` if the input is empty.
    pub fn arbitrate(commands: &[PrioritizedCommand]) -> Option<ArbitrationOutcome> {
        if commands.is_empty() {
            return None;
        }
        let mut best_idx = 0usize;
        for i in 1..commands.len() {
            if Self::beats(&commands[i], &commands[best_idx]) {
                best_idx = i;
            }
        }
        let winner = commands[best_idx];
        let mut overridden = Vec::new();
        for (i, c) in commands.iter().enumerate() {
            if i != best_idx {
                overridden.push(c.source.id);
            }
        }
        let reason = if commands.iter().filter(|c| c.source.tier == winner.source.tier).count() > 1
        {
            ReasonKind::ArbitratedTieBreak
        } else {
            ReasonKind::ArbitratedToHigherTier
        };
        Some(ArbitrationOutcome { winner, reason, overridden_sources: overridden })
    }

    /// True if `a` should win over `b` under the documented rules.
    fn beats(a: &PrioritizedCommand, b: &PrioritizedCommand) -> bool {
        use core::cmp::Ordering;
        match a.source.tier.cmp(&b.source.tier) {
            Ordering::Greater => true,
            Ordering::Less => false,
            Ordering::Equal => {
                // same tier: most recent wins
                match a.envelope.timestamp_ms.cmp(&b.envelope.timestamp_ms) {
                    Ordering::Greater => true,
                    Ordering::Less => false,
                    Ordering::Equal => a.source.id < b.source.id, // deterministic tie-break
                }
            }
        }
    }

    /// Arbitrate a mixed set of commands, grouping by actuator first.
    pub fn arbitrate_by_actuator(
        commands: &[PrioritizedCommand],
    ) -> Vec<(ActuatorId, ArbitrationOutcome)> {
        let mut actuators: Vec<ActuatorId> = commands.iter().map(|c| c.envelope.actuator).collect();
        actuators.sort_unstable();
        actuators.dedup();
        let mut out = Vec::new();
        for act in actuators {
            let group: Vec<PrioritizedCommand> =
                commands.iter().filter(|c| c.envelope.actuator == act).copied().collect();
            if let Some(outcome) = Self::arbitrate(&group) {
                out.push((act, outcome));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_control_action::{ActuatorState, CommandValue, RequestId};

    fn cmd(
        source_id: u32,
        tier: PriorityTier,
        actuator: ActuatorId,
        ts: u64,
    ) -> PrioritizedCommand {
        PrioritizedCommand {
            source: ControlSource { id: source_id, tier },
            envelope: CommandEnvelope::new(
                actuator,
                CommandValue::discrete(ActuatorState::On),
                source_id as RequestId,
                ts,
            ),
        }
    }

    #[test]
    fn empty_yields_none() {
        assert_eq!(ArbitrationEngine::arbitrate(&[]), None);
    }

    #[test]
    fn higher_tier_wins() {
        let cmds = [
            cmd(1, PriorityTier::Schedule, 1, 0),
            cmd(2, PriorityTier::AutoOptimization, 1, 0),
            cmd(3, PriorityTier::Safety, 1, 0),
        ];
        let out = ArbitrationEngine::arbitrate(&cmds).unwrap();
        assert_eq!(out.winner.source.tier, PriorityTier::Safety);
        assert_eq!(out.reason, ReasonKind::ArbitratedToHigherTier);
        assert_eq!(out.overridden_sources.len(), 2);
    }

    #[test]
    fn safety_beats_manual() {
        let cmds = [cmd(1, PriorityTier::ManualOverride, 1, 0), cmd(2, PriorityTier::Safety, 1, 0)];
        let out = ArbitrationEngine::arbitrate(&cmds).unwrap();
        assert_eq!(out.winner.source.tier, PriorityTier::Safety);
    }

    #[test]
    fn tie_breaks_on_timestamp() {
        let cmds = [
            cmd(1, PriorityTier::ManualOverride, 1, 100),
            cmd(2, PriorityTier::ManualOverride, 1, 200), // newer wins
        ];
        let out = ArbitrationEngine::arbitrate(&cmds).unwrap();
        assert_eq!(out.winner.source.id, 2);
        assert_eq!(out.reason, ReasonKind::ArbitratedTieBreak);
    }

    #[test]
    fn full_tie_breaks_on_source_id() {
        let cmds = [cmd(5, PriorityTier::Schedule, 1, 0), cmd(2, PriorityTier::Schedule, 1, 0)];
        let out = ArbitrationEngine::arbitrate(&cmds).unwrap();
        assert_eq!(out.winner.source.id, 2);
    }

    #[test]
    fn grouping_by_actuator() {
        let cmds = [
            cmd(1, PriorityTier::Safety, 1, 0),
            cmd(2, PriorityTier::Schedule, 1, 0),
            cmd(3, PriorityTier::AutoOptimization, 2, 0),
        ];
        let out = ArbitrationEngine::arbitrate_by_actuator(&cmds);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0, 1);
        assert_eq!(out[0].1.winner.source.tier, PriorityTier::Safety);
        assert_eq!(out[1].0, 2);
    }
}
