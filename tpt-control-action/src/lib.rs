//! # tpt-control-action
//!
//! Foundation crate for tpt-rust5. Defines the core traits and data model for physical
//! actuators (discrete `ON/OFF/FAULT` states and continuous `0–100%` setpoints) and the
//! internal command-envelope representation that the rest of the safety guardrails operate on.
//!
//! This crate is `no_std` + `alloc`: pure logic with no inherent I/O, deployable on
//! constrained / PLC-like controllers.
//!
//! ## TUP integration note
//!
//! The upstream [`tpt-protocol`] TUP schema (`SPEC-TUP.md`) is currently telemetry-only and has
//! no write-direction "Command Envelope" yet. The internal [`CommandEnvelope`] type defined here
//! is the single mapping boundary: when the upstream command schema lands, only the `tup` module
//! needs to change.

#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

use core::fmt;

/// Identifier for a physical actuator (valve, pump, motor, heater, …).
pub type ActuatorId = u32;

/// Identifier for a control request / command envelope.
pub type RequestId = u64;

/// Discrete actuator state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ActuatorState {
    /// Actuator energized / running.
    On,
    /// Actuator de-energized / stopped.
    Off,
    /// Actuator in a faulted condition — must not be commanded until cleared.
    Fault,
}

impl ActuatorState {
    /// Returns `true` for a commandable (non-fault) state.
    pub fn is_commandable(self) -> bool {
        self != ActuatorState::Fault
    }
}

impl fmt::Display for ActuatorState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            ActuatorState::On => "ON",
            ActuatorState::Off => "OFF",
            ActuatorState::Fault => "FAULT",
        };
        f.write_str(s)
    }
}

/// Engineering units for a continuous setpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Units {
    /// Percent of full scale (`0..=100`).
    Percent,
    /// Degrees Celsius.
    Celsius,
    /// Bar (gauge pressure).
    Bar,
    /// Liters per second.
    LitresPerSecond,
    /// Unitless / raw count.
    Raw,
}

impl fmt::Display for Units {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Units::Percent => "%",
            Units::Celsius => "°C",
            Units::Bar => "bar",
            Units::LitresPerSecond => "L/s",
            Units::Raw => "raw",
        };
        f.write_str(s)
    }
}

/// Inclusive saturation bounds for a continuous value.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Bounds {
    pub min: f64,
    pub max: f64,
}

impl Bounds {
    /// The canonical `0..=100` percent bounds.
    pub const PERCENT: Bounds = Bounds { min: 0.0, max: 100.0 };

    /// Returns `true` if `value` lies within `[min, max]`.
    pub fn contains(&self, value: f64) -> bool {
        value >= self.min && value <= self.max
    }

    /// Clamp `value` into `[min, max]`.
    pub fn clamp(&self, value: f64) -> f64 {
        if value < self.min {
            self.min
        } else if value > self.max {
            self.max
        } else {
            value
        }
    }

    /// Validate that `min <= max`.
    pub fn is_valid(&self) -> bool {
        self.min <= self.max
    }
}

/// A continuous setpoint with its engineering bounds and units.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Setpoint {
    pub value: f64,
    pub bounds: Bounds,
    pub units: Units,
}

impl Setpoint {
    /// Construct a percent setpoint (bounds `0..=100`).
    pub fn percent(value: f64) -> Self {
        Setpoint { value, bounds: Bounds::PERCENT, units: Units::Percent }
    }

    /// Returns `true` if the value is within its bounds.
    pub fn in_bounds(&self) -> bool {
        self.bounds.contains(self.value)
    }

    /// Clamp the value into its bounds, returning the (possibly changed) setpoint.
    pub fn clamped(&self) -> Self {
        Setpoint { value: self.bounds.clamp(self.value), bounds: self.bounds, units: self.units }
    }
}

/// A command value: either a discrete state or a continuous setpoint.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CommandValue {
    Discrete(ActuatorState),
    Continuous(Setpoint),
}

impl CommandValue {
    /// Construct a discrete command.
    pub fn discrete(state: ActuatorState) -> Self {
        CommandValue::Discrete(state)
    }

    /// Construct a continuous (percent) command.
    pub fn continuous(value: f64) -> Self {
        CommandValue::Continuous(Setpoint::percent(value))
    }

    /// Returns the discrete state, if this is a discrete command.
    pub fn as_discrete(&self) -> Option<ActuatorState> {
        match self {
            CommandValue::Discrete(s) => Some(*s),
            CommandValue::Continuous(_) => None,
        }
    }

    /// Returns the continuous setpoint, if this is a continuous command.
    pub fn as_continuous(&self) -> Option<Setpoint> {
        match self {
            CommandValue::Continuous(s) => Some(*s),
            CommandValue::Discrete(_) => None,
        }
    }
}

/// The internal command envelope — the common currency of every crate in tpt-rust5.
///
/// It is intentionally decoupled from any concrete wire format so that all downstream safety
/// logic can be built and tested without the (not-yet-existing) upstream TUP Command Envelope.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CommandEnvelope {
    pub actuator: ActuatorId,
    pub value: CommandValue,
    pub request_id: RequestId,
    pub timestamp_ms: u64,
}

impl CommandEnvelope {
    /// Construct a new envelope.
    pub fn new(
        actuator: ActuatorId,
        value: CommandValue,
        request_id: RequestId,
        timestamp_ms: u64,
    ) -> Self {
        CommandEnvelope { actuator, value, request_id, timestamp_ms }
    }
}

/// Errors produced by this crate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ActionError {
    /// A continuous setpoint value fell outside its declared bounds.
    OutOfBounds { value: f64, min: f64, max: f64 },
    /// A discrete command targeted a faulted actuator.
    ActuatorFaulted(ActuatorId),
    /// TUP mapping failure.
    TupMapping(&'static str),
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActionError::OutOfBounds { value, min, max } => {
                write!(f, "setpoint {value} out of bounds [{min}, {max}]")
            }
            ActionError::ActuatorFaulted(id) => write!(f, "actuator {id} is faulted"),
            ActionError::TupMapping(msg) => write!(f, "TUP mapping error: {msg}"),
        }
    }
}

/// Core trait for a discrete actuator (ON/OFF/FAULT).
pub trait DiscreteActuator {
    /// Current reported state of the actuator.
    fn state(&self) -> ActuatorState;
    /// Whether the actuator is currently commandable (not faulted).
    fn is_commandable(&self) -> bool {
        self.state().is_commandable()
    }
}

/// Core trait for a continuous setpoint actuator, carrying bounds/units metadata.
pub trait ContinuousActuator {
    /// Declared bounds and units for this actuator's setpoint.
    fn setpoint_metadata(&self) -> Setpoint;
    /// Validate a proposed continuous value against the actuator's bounds.
    fn validate(&self, value: f64) -> Result<Setpoint, ActionError> {
        let meta = self.setpoint_metadata();
        if meta.bounds.contains(value) {
            Ok(Setpoint { value, bounds: meta.bounds, units: meta.units })
        } else {
            Err(ActionError::OutOfBounds { value, min: meta.bounds.min, max: meta.bounds.max })
        }
    }
}

/// Validate a [`CommandEnvelope`] for a given actuator's metadata.
///
/// Returns the (clamped) envelope if valid, or an [`ActionError`].
pub fn validate_envelope(
    envelope: &CommandEnvelope,
    continuous_meta: Option<Setpoint>,
    discrete_state: Option<ActuatorState>,
) -> Result<CommandEnvelope, ActionError> {
    let validated = match (envelope.value, continuous_meta, discrete_state) {
        (CommandValue::Continuous(sp), Some(meta), _) => {
            if !meta.bounds.contains(sp.value) {
                return Err(ActionError::OutOfBounds {
                    value: sp.value,
                    min: meta.bounds.min,
                    max: meta.bounds.max,
                });
            }
            envelope
        }
        (CommandValue::Discrete(state), _, Some(current)) => {
            if state != ActuatorState::Fault && current == ActuatorState::Fault {
                return Err(ActionError::ActuatorFaulted(envelope.actuator));
            }
            envelope
        }
        _ => envelope,
    };
    Ok(*validated)
}

/// Live hardware state interface, consumed by the safety layers.
///
/// The upstream [`tpt-rust3`] `tpt-state-snapshot` crate will implement this trait; the safety
/// crates depend only on the trait, so the whole workspace builds and tests against mock state.
pub mod live_state {
    use crate::ActuatorId;
    use alloc::vec::Vec;

    /// Identifier for a sensor / telemetry channel.
    pub type SensorId = u32;

    /// Health of a sensor reading.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum SensorHealth {
        /// Value is fresh and可信.
        Healthy,
        /// Value is present but stale (older than the freshness threshold).
        Stale,
        /// Sensor failed / value unavailable.
        Failed,
    }

    /// A single live sensor reading.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct SensorReading {
        pub value: f64,
        pub health: SensorHealth,
        pub timestamp_ms: u64,
    }

    impl SensorReading {
        pub fn healthy(value: f64, timestamp_ms: u64) -> Self {
            SensorReading { value, health: SensorHealth::Healthy, timestamp_ms }
        }
        /// Returns `true` if the reading can be trusted for safety evaluation.
        pub fn is_usable(&self) -> bool {
            self.health == SensorHealth::Healthy
        }
    }

    /// Live state provider — read access to current sensor/actuator state.
    pub trait LiveStateProvider {
        /// Read a sensor channel, if available.
        fn read_sensor(&self, sensor: SensorId) -> Option<SensorReading>;
        /// Read the current reported state of an actuator, if available.
        fn read_actuator(&self, actuator: ActuatorId) -> Option<crate::ActuatorState>;
        /// List of sensor ids currently known to the provider (for completeness checks).
        fn known_sensors(&self) -> Vec<SensorId> {
            Vec::new()
        }
    }
}

/// Stand-in TUP command envelope (write direction) until the upstream schema lands.
///
/// This is a deliberately simple wire-shaped representation. The `From`/`TryFrom` impls below are
/// the single mapping surface that will be replaced once `tpt-protocol`'s real Command Envelope
/// is published.
pub mod tup {
    use super::*;

    /// Wire-shaped TUP command (telemetry-outbound-compatible stand-in).
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct TupCommandEnvelope {
        pub actuator: ActuatorId,
        /// Signed command: negative => discrete OFF, 0 => discrete ON marker is encoded via
        /// `discrete`, positive => continuous percent. Kept flat for wire simplicity.
        pub discrete: Option<bool>,
        pub continuous_percent: Option<f64>,
        pub request_id: RequestId,
        pub timestamp_ms: u64,
    }

    impl From<CommandEnvelope> for TupCommandEnvelope {
        fn from(e: CommandEnvelope) -> Self {
            match e.value {
                CommandValue::Discrete(s) => TupCommandEnvelope {
                    actuator: e.actuator,
                    discrete: Some(s == ActuatorState::On),
                    continuous_percent: None,
                    request_id: e.request_id,
                    timestamp_ms: e.timestamp_ms,
                },
                CommandValue::Continuous(sp) => TupCommandEnvelope {
                    actuator: e.actuator,
                    discrete: None,
                    continuous_percent: Some(sp.value),
                    request_id: e.request_id,
                    timestamp_ms: e.timestamp_ms,
                },
            }
        }
    }

    impl TryFrom<TupCommandEnvelope> for CommandEnvelope {
        type Error = ActionError;
        fn try_from(t: TupCommandEnvelope) -> Result<Self, Self::Error> {
            let value = match (t.discrete, t.continuous_percent) {
                (Some(on), None) => {
                    CommandValue::Discrete(if on { ActuatorState::On } else { ActuatorState::Off })
                }
                (None, Some(p)) => CommandValue::Continuous(Setpoint::percent(p)),
                _ => {
                    return Err(ActionError::TupMapping(
                        "exactly one of discrete/continuous must be set",
                    ))
                }
            };
            Ok(CommandEnvelope::new(t.actuator, value, t.request_id, t.timestamp_ms))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_clamp_and_contains() {
        let b = Bounds { min: 10.0, max: 90.0 };
        assert!(b.contains(10.0));
        assert!(b.contains(90.0));
        assert!(!b.contains(9.9));
        assert_eq!(b.clamp(200.0), 90.0);
        assert_eq!(b.clamp(-5.0), 10.0);
        assert_eq!(b.clamp(50.0), 50.0);
    }

    #[test]
    fn setpoint_in_bounds_and_clamp() {
        let sp = Setpoint::percent(120.0);
        assert!(!sp.in_bounds());
        assert_eq!(sp.clamped().value, 100.0);
        let ok = Setpoint::percent(42.0);
        assert!(ok.in_bounds());
        assert_eq!(ok.clamped().value, 42.0);
    }

    #[test]
    fn discrete_trait_and_fault() {
        struct Pump {
            state: ActuatorState,
        }
        impl DiscreteActuator for Pump {
            fn state(&self) -> ActuatorState {
                self.state
            }
        }
        let pump = Pump { state: ActuatorState::Fault };
        assert!(!pump.is_commandable());
        assert!(ActuatorState::On.is_commandable());
    }

    #[test]
    fn continuous_trait_validate() {
        struct Valve;
        impl ContinuousActuator for Valve {
            fn setpoint_metadata(&self) -> Setpoint {
                Setpoint::percent(0.0)
            }
        }
        let v = Valve;
        assert!(v.validate(50.0).is_ok());
        assert!(v.validate(150.0).is_err());
    }

    #[test]
    fn validate_envelope_blocks_fault_command_to_faulted_actuator() {
        let env = CommandEnvelope::new(1, CommandValue::discrete(ActuatorState::On), 1, 0);
        // command to ON while actuator currently Fault => rejected
        let r = validate_envelope(&env, None, Some(ActuatorState::Fault));
        assert_eq!(r, Err(ActionError::ActuatorFaulted(1)));
        // command to ON while actuator currently Off => ok
        let r2 = validate_envelope(&env, None, Some(ActuatorState::Off));
        assert!(r2.is_ok());
    }

    #[test]
    fn validate_envelope_bounds_continuous() {
        let env = CommandEnvelope::new(2, CommandValue::continuous(150.0), 1, 0);
        let meta = Setpoint::percent(0.0);
        assert_eq!(
            validate_envelope(&env, Some(meta), None),
            Err(ActionError::OutOfBounds { value: 150.0, min: 0.0, max: 100.0 })
        );
    }

    #[test]
    fn tup_round_trip_discrete() {
        let env = CommandEnvelope::new(3, CommandValue::discrete(ActuatorState::Off), 7, 123);
        let wire: tup::TupCommandEnvelope = env.into();
        let back: CommandEnvelope = wire.try_into().unwrap();
        assert_eq!(env, back);
    }

    #[test]
    fn tup_round_trip_continuous() {
        let env = CommandEnvelope::new(4, CommandValue::continuous(55.5), 9, 200);
        let wire: tup::TupCommandEnvelope = env.into();
        let back: CommandEnvelope = wire.try_into().unwrap();
        assert_eq!(env, back);
    }

    #[test]
    fn tup_mapping_rejects_both_set() {
        let bad = tup::TupCommandEnvelope {
            actuator: 1,
            discrete: Some(true),
            continuous_percent: Some(10.0),
            request_id: 1,
            timestamp_ms: 0,
        };
        assert!(CommandEnvelope::try_from(bad).is_err());
    }

    #[test]
    fn command_value_accessors() {
        assert_eq!(
            CommandValue::discrete(ActuatorState::On).as_discrete(),
            Some(ActuatorState::On)
        );
        assert_eq!(CommandValue::continuous(30.0).as_continuous().unwrap().value, 30.0);
        assert_eq!(CommandValue::discrete(ActuatorState::On).as_continuous(), None);
    }
}
