# `radio-cat-rs` Design Notes

## Purpose

`radio-cat-rs` is a Rust library for controlling amateur radios, transceivers, receivers, and related CAT-controllable radio devices.

The library is not intended to be a thin wrapper around request/response CAT commands. Instead, it should provide a stateful, asynchronous control model:

- the library owns the radio connection
- the library continuously maintains an internal state cache
- applications subscribe to state updates
- applications send commands through an async API
- radio-specific VFO/protocol details are hidden behind a normalized public model

The primary intended consumer is a program such as Log73, which may control multiple radios simultaneously.

---

# Core Design Principle

Applications should not need to poll radios directly.

Instead, every radio connection should expose asynchronous state updates, regardless of whether the underlying radio supports native unsolicited CAT updates.

The source of an update may be:

- native unsolicited radio messages
- responses to commands
- periodic polling
- manual refresh
- optimistic local updates, if used

From the application’s perspective, all of these become a single async state/update stream.

---

# Runtime Model

Each connected radio is modeled internally as an actor:

```text
Application
   |
   | commands
   v
RadioHandle
   |
   | mpsc command channel
   v
RadioTask / actor
   |
   +-- transport reader
   +-- transport writer
   +-- protocol parser
   +-- command queue
   +-- poll scheduler
   +-- state reducer
   +-- update broadcaster
   |
   v
Serial / TCP / other transport
```

The application should never directly own the serial port or transport after connection. The radio task owns the transport and serializes access to it.

---

# Async Update Strategy

Every radio should expose the same async update API.

For radios that support native asynchronous updates, the driver should use them.

For radios that do not support native updates, the library should simulate async updates using a poll interval.

For radios that support some native updates but not all relevant state, the driver may use a hybrid strategy.

```rust
pub enum UpdateStrategy {
    NativeAsync,
    Polling {
        interval: Duration,
    },
    Hybrid {
        interval: Duration,
    },
}
```

This should be mostly internal. The public promise is:

> Every `radio-cat-rs` connection provides an async state stream.

Not:

> Every physical radio provides native async notifications.

---

# Public State Model

The public state model should be signal-path oriented, not VFO-oriented.

The application should not need to know whether the radio internally uses VFO A/B, Main/Sub, RXA/RXB, memory channels, or some other model.

The normalized state should track:

- main receiver
- optional sub receiver
- optional transmitter
- RIT/XIT state
- optional keyer state
- connection state

## State Structure

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RadioState {
    pub connection: ConnectionState,

    pub main_rx: ReceiverState,
    pub sub_rx: Option<ReceiverState>,

    pub tx: Option<TransmitterState>,

    pub rit_xit: RitXitState,

    pub keyer: Option<KeyerState>,
}
```

---

# Receiver State

RF, filter, and receive-side DSP state should live inside `ReceiverState`, because on some radios these settings are separate per VFO or receiver path.

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiverState {
    pub frequency: Option<Frequency>,
    pub mode: Option<Mode>,

    pub filter: ReceiverFilterState,
    pub rf: ReceiverRfState,
}
```

`Frequency` should use the existing frequency type already present in `radio-cat-rs`.

Although `u64` is preferred as the underlying representation for frequency, the public model should reuse the crate’s existing frequency type rather than introduce a second competing type.

## Receiver Filter State

The receiver filter state includes both bandwidth and shift.

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiverFilterState {
    pub bandwidth_hz: Option<u16>,
    pub shift_hz: Option<u16>,
}
```

Both values are optional because a radio may support one and not the other, or the value may not yet be known.

## Receiver RF State

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiverRfState {
    pub preamp: Option<LeveledSetting>,
    pub attenuator: Option<LeveledSetting>,
    pub noise_blanker: Option<LeveledSetting>,
    pub noise_reduction: Option<LeveledSetting>,
    pub auto_notch: Option<bool>,
}
```

These are receiver-path properties, not global radio properties.

For example, on a dual-receive radio, the main receiver and sub receiver may have different preamp, attenuator, noise blanker, noise reduction, filter bandwidth, or filter shift settings.

---

# Transmitter State

The transmitter is optional because the library may control a receive-only radio or SDR.

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransmitterState {
    pub frequency: Option<Frequency>,
    pub mode: Option<Mode>,
    pub power_deci_mw: Option<u32>,
    pub transmitting: Option<bool>,
    pub split: Option<bool>,
}
```

`tx: None` means the device has no transmitter or the driver exposes no transmitter capability.

`tx: Some(...)` with individual fields set to `None` means the transmitter exists, but that particular value is currently unknown.

Power is represented as `0.1 mW` units using `u32`.

---

# RIT/XIT State

RIT/XIT offset is limited to ±9999 Hz, so `i16` is sufficient.

A newtype should be used to enforce the normalized range.

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RitXitState {
    pub rit_enabled: Option<bool>,
    pub xit_enabled: Option<bool>,
    pub offset_hz: Option<RitXitOffsetHz>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RitXitOffsetHz(i16);

impl RitXitOffsetHz {
    pub const MIN: i16 = -9999;
    pub const MAX: i16 = 9999;

    pub fn new(value: i16) -> Result<Self, RangeError> {
        if (Self::MIN..=Self::MAX).contains(&value) {
            Ok(Self(value))
        } else {
            Err(RangeError)
        }
    }

    pub fn as_hz(self) -> i16 {
        self.0
    }
}
```

---

# Keyer State

The keyer is optional because not all radios have or expose an internal keyer.

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyerState {
    pub speed_wpm: Option<u8>,
}
```

---

# Leveled Settings

Several radio settings have both an enabled state and a level.

Examples:

- noise blanker
- noise reduction
- preamp
- attenuator

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeveledSetting {
    pub enabled: Option<bool>,
    pub level: Option<u8>,
}
```

This supports radios where:

- the enabled state is known but the level is unknown
- the level is known but the enabled state is unknown
- the feature is unsupported
- the feature is off but still has a remembered level

---

# Unknown vs Unsupported

The state model should distinguish between unsupported features and unknown values.

For optional components:

```rust
sub_rx: None
```

means no sub receiver is exposed.

```rust
sub_rx: Some(ReceiverState {
    frequency: None,
    mode: None,
    filter: ReceiverFilterState {
        bandwidth_hz: None,
        shift_hz: None,
    },
    rf: ReceiverRfState { ... },
})
```

means the sub receiver exists, but some state has not yet been learned.

Similarly:

```rust
tx: None
```

means receiver-only or no exposed transmitter capability.

```rust
tx: Some(TransmitterState {
    frequency: None,
    mode: None,
    power_deci_mw: None,
    transmitting: None,
    split: None,
})
```

means a transmitter exists, but its state is partially unknown.

---

# State Updates

The library should expose both:

1. latest state snapshots
2. categorized update events

## Latest State

Use a `watch` channel for the latest state.

```rust
pub type SharedRadioState = Arc<RadioState>;

pub fn subscribe_state(&self) -> watch::Receiver<SharedRadioState>;
```

This lets new subscribers immediately receive the current known state.

Using `Arc<RadioState>` avoids excessive copying across threads and subscribers.

## State Update Events

Use a separate update stream for change metadata.

```rust
pub struct StateUpdate {
    pub changes: ChangeFlags,
    pub fields: SmallVec<[StateField; 4]>,
    pub source: UpdateSource,
    pub state: Arc<RadioState>,
}
```

The update carries:

- broad change categories
- optional precise field identifiers
- update source
- a shared snapshot of the resulting state

---

# Change Flags

Change flags should identify what category of state changed.

The flags should match the signal-path model rather than VFO internals.

```rust
bitflags::bitflags! {
    pub struct ChangeFlags: u32 {
        const NONE                  = 0;

        const MAIN_RX_FREQ          = 1 << 0;
        const MAIN_RX_MODE          = 1 << 1;
        const MAIN_RX_FILTER_BW     = 1 << 2;
        const MAIN_RX_FILTER_SHIFT  = 1 << 3;
        const MAIN_RX_RF            = 1 << 4;

        const SUB_RX                = 1 << 5;
        const SUB_RX_FREQ           = 1 << 6;
        const SUB_RX_MODE           = 1 << 7;
        const SUB_RX_FILTER_BW      = 1 << 8;
        const SUB_RX_FILTER_SHIFT   = 1 << 9;
        const SUB_RX_RF             = 1 << 10;

        const TX                    = 1 << 11;
        const TX_FREQ               = 1 << 12;
        const TX_MODE               = 1 << 13;
        const TX_POWER              = 1 << 14;
        const PTT                   = 1 << 15;
        const SPLIT                 = 1 << 16;

        const RIT_XIT               = 1 << 17;
        const KEYER                 = 1 << 18;
        const CONNECTION            = 1 << 19;

        const OTHER                 = 1 << 31;

        const FREQUENCY =
            Self::MAIN_RX_FREQ.bits()
            | Self::SUB_RX_FREQ.bits()
            | Self::TX_FREQ.bits();

        const MODE =
            Self::MAIN_RX_MODE.bits()
            | Self::SUB_RX_MODE.bits()
            | Self::TX_MODE.bits();

        const FILTER =
            Self::MAIN_RX_FILTER_BW.bits()
            | Self::MAIN_RX_FILTER_SHIFT.bits()
            | Self::SUB_RX_FILTER_BW.bits()
            | Self::SUB_RX_FILTER_SHIFT.bits();

        const RECEIVER =
            Self::MAIN_RX_FREQ.bits()
            | Self::MAIN_RX_MODE.bits()
            | Self::MAIN_RX_FILTER_BW.bits()
            | Self::MAIN_RX_FILTER_SHIFT.bits()
            | Self::MAIN_RX_RF.bits()
            | Self::SUB_RX.bits()
            | Self::SUB_RX_FREQ.bits()
            | Self::SUB_RX_MODE.bits()
            | Self::SUB_RX_FILTER_BW.bits()
            | Self::SUB_RX_FILTER_SHIFT.bits()
            | Self::SUB_RX_RF.bits();

        const TRANSMITTER =
            Self::TX.bits()
            | Self::TX_FREQ.bits()
            | Self::TX_MODE.bits()
            | Self::TX_POWER.bits()
            | Self::PTT.bits()
            | Self::SPLIT.bits();
    }
}
```

Applications can then cheaply filter updates:

```rust
if update.changes.intersects(ChangeFlags::FREQUENCY) {
    // update bandmap, frequency display, logging context, etc.
}

if update.changes.intersects(ChangeFlags::MAIN_RX_FREQ | ChangeFlags::MAIN_RX_MODE) {
    // update main operating context
}

if update.changes.intersects(ChangeFlags::FILTER) {
    // update filter display
}

if update.changes.intersects(ChangeFlags::TX_FREQ | ChangeFlags::SPLIT) {
    // update transmit indicator
}
```

---

# Field-Level Change Details

Broad flags are useful, but the reducer already knows exactly which field changed.

The update event may also include precise field identifiers.

```rust
pub enum StateField {
    MainRxFrequency,
    MainRxMode,
    MainRxFilterBandwidth,
    MainRxFilterShift,
    MainRxPreamp,
    MainRxAttenuator,
    MainRxNoiseBlanker,
    MainRxNoiseReduction,
    MainRxAutoNotch,

    SubRxPresent,
    SubRxFrequency,
    SubRxMode,
    SubRxFilterBandwidth,
    SubRxFilterShift,
    SubRxPreamp,
    SubRxAttenuator,
    SubRxNoiseBlanker,
    SubRxNoiseReduction,
    SubRxAutoNotch,

    TxPresent,
    TxFrequency,
    TxMode,
    TxPower,
    Transmitting,
    Split,

    RitEnabled,
    XitEnabled,
    RitXitOffset,

    KeyerSpeed,

    Connection,
    Other(&'static str),
}
```

---

# Internal State Reducer

Drivers should not directly broadcast state changes.

Instead, protocol frames and poll responses should produce internal patches.

```rust
pub enum StatePatch {
    MainRxFrequency(Frequency),
    MainRxMode(Mode),
    MainRxFilterBandwidth(u16),
    MainRxFilterShift(u16),

    SubRxPresent(bool),
    SubRxFrequency(Frequency),
    SubRxMode(Mode),
    SubRxFilterBandwidth(u16),
    SubRxFilterShift(u16),

    TxPresent(bool),
    TxFrequency(Frequency),
    TxMode(Mode),
    TxPowerDeciMw(u32),
    Transmitting(bool),
    Split(bool),

    RitEnabled(bool),
    XitEnabled(bool),
    RitXitOffset(RitXitOffsetHz),

    KeyerSpeed(u8),

    Connection(ConnectionState),
}
```

The reducer applies a patch to `RadioState` and returns a `ChangeSet`.

```rust
pub struct ChangeSet {
    pub flags: ChangeFlags,
    pub fields: SmallVec<[StateField; 4]>,
}
```

Pipeline:

```text
CAT frame / poll response
        ↓
Protocol driver
        ↓
StatePatch
        ↓
State reducer
        ↓
ChangeSet
        ↓
StateUpdate
        ↓
Application subscribers
```

Only meaningful changes should be emitted. Poll responses that confirm unchanged state should not generate update events.

---

# Update Sources

Each update should record where it came from.

```rust
pub enum UpdateSource {
    Native,
    Poll,
    CommandResponse,
    ManualRefresh,
    Optimistic,
}
```

This is useful for debugging, UI freshness indicators, and understanding radio behavior.

---

# Polling

Polling is the fallback mechanism for radios without native async updates.

Polling should be internal to the library. Applications should subscribe to state instead of writing their own polling loops.

Polling should be per capability, not one single fixed loop.

For example:

- frequency: faster
- mode: moderate
- PTT: moderate
- split: moderate
- filter bandwidth and shift: moderate or slow
- RF/DSP controls: slower
- keyer speed: slower or on demand

```rust
pub struct PollingPlan {
    pub items: Vec<PollItem>,
}

pub struct PollItem {
    pub command: RadioCommand,
    pub interval: Duration,
    pub priority: PollPriority,
}
```

User commands should take priority over background polling.

Suggested priority order:

1. urgent user commands such as PTT, CW/RTTY keying, frequency set
2. command responses already in flight
3. high-priority polling such as frequency, mode, PTT
4. lower-priority polling such as split, filters, RF/DSP state, keyer state

---

# Commands

The public API should be semantic and signal-path oriented.

Examples:

```rust
radio.set_main_frequency(freq).await?;
radio.set_main_mode(mode).await?;
radio.set_main_filter_bandwidth(hz).await?;
radio.set_main_filter_shift(hz).await?;

radio.set_sub_frequency(freq).await?;
radio.set_sub_mode(mode).await?;
radio.set_sub_filter_bandwidth(hz).await?;
radio.set_sub_filter_shift(hz).await?;

radio.set_tx_frequency(freq).await?;
radio.set_tx_mode(mode).await?;
radio.set_split(true).await?;
radio.set_ptt(true).await?;

radio.set_rit_enabled(true).await?;
radio.set_xit_enabled(false).await?;
radio.set_rit_xit_offset(offset).await?;

radio.set_keyer_speed(wpm).await?;
```

Unsupported commands should return a structured unsupported-capability error.

---

# Command Completion Semantics

Setter commands have multiple possible meanings of “complete”:

1. command was sent to the radio
2. radio accepted or acknowledged the command
3. library observed the expected state change

The library should define this distinction internally and may expose it later.

```rust
pub enum CommandCompletion {
    Sent,
    Accepted,
    Observed,
}
```

Default behavior should probably be `Accepted` or `Observed`, depending on radio capability and command type.

For logging and contesting applications, `Observed` is often the most useful semantic because it confirms the state cache changed.

---

# Capabilities

Capabilities should reflect the normalized public model.

```rust
pub struct RadioCapabilities {
    pub main_rx: ReceiverCapabilities,
    pub sub_rx: Option<ReceiverCapabilities>,
    pub tx: Option<TransmitterCapabilities>,

    pub rit_xit: Capability,
    pub keyer: Option<KeyerCapabilities>,

    pub state_updates: StateUpdateCapability,
}
```

```rust
pub struct ReceiverCapabilities {
    pub frequency: Capability,
    pub mode: Capability,
    pub filter_bandwidth: Capability,
    pub filter_shift: Capability,
    pub rf: ReceiverRfCapabilities,
}
```

```rust
pub enum StateUpdateCapability {
    Native,
    Polling,
    Hybrid,
}
```

The application can assume async state delivery exists, but it can still inspect whether the underlying implementation is native, polling, or hybrid.

---

# Thread and Runtime Model

A single Tokio runtime should be able to handle many radios.

Six radios should not require six OS threads. CAT traffic is small, and polling/read/write operations should be lightweight if implemented with non-blocking async transports.

However, the crate should not force all radio tasks to run on the caller’s main runtime.

## Recommended API

Provide both a convenience API and a manual task API.

```rust
impl Radio {
    pub async fn connect(config: RadioConfig) -> Result<Radio>;

    pub async fn build(config: RadioConfig) -> Result<(Radio, RadioTask)>;
}
```

`connect` spawns the task on the current Tokio runtime:

```rust
let radio = Radio::connect(config).await?;
```

`build` allows the application to decide where the task runs:

```rust
let (radio, task) = Radio::build(config).await?;

tokio::spawn(task.run());
```

An application may also move the task to a dedicated runtime thread:

```rust
let (radio, task) = Radio::build(config).await?;

std::thread::spawn(move || {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(task.run());
});
```

The library should not create a hidden global runtime.

---

# Cross-Thread Copying

Use shared immutable snapshots to avoid excessive copying.

Recommended channel model:

```rust
watch::Receiver<Arc<RadioState>>      // latest state snapshot
broadcast::Receiver<StateUpdate>      // change metadata + Arc<RadioState>
mpsc::Sender<CommandEnvelope>         // commands into radio actor
oneshot::Sender<Result<T>>            // command results back
```

The authoritative mutable state stays inside the radio actor.

On each meaningful change, the actor creates a new `Arc<RadioState>` snapshot and publishes it.

This avoids cloning the full state for every subscriber.

---

# Public Handle

The public handle should be cheap, cloneable, and safe to move across threads.

```rust
#[derive(Clone)]
pub struct Radio {
    command_tx: mpsc::Sender<CommandEnvelope>,
    state_rx: watch::Receiver<Arc<RadioState>>,
    update_tx: broadcast::Sender<StateUpdate>,
}
```

The handle should be `Send + Sync`.

---

# Transport and Protocol Separation

Transport should be separate from protocol.

Transports may include:

- serial
- TCP
- UDP, if needed later
- virtual or remote transports

Protocol drivers should handle:

- framing
- parsing
- command construction
- response classification
- native update handling
- polling plan
- capabilities

```rust
pub trait RadioDriver: Send + 'static {
    fn build_command(&self, command: RadioCommand) -> Result<Vec<u8>>;

    fn parse_bytes(&mut self, bytes: &[u8]) -> Result<Vec<ProtocolMessage>>;

    fn initial_queries(&self) -> Vec<RadioCommand>;

    fn polling_plan(&self) -> PollingPlan;

    fn capabilities(&self) -> RadioCapabilities;
}
```

```rust
pub enum ProtocolMessage {
    StatePatch(StatePatch),
    CommandResponse(CommandResponse),
    Ack,
    Nak,
    Unknown,
}
```

---

# Design Summary

`radio-cat-rs` should be a stateful async radio controller.

The core abstraction is:

```text
Radio = command sink + state source + update source
```

Not:

```text
Radio = collection of request/response CAT functions
```

Applications should work with a normalized state model based on:

```text
main receiver
optional sub receiver
optional transmitter
RIT/XIT
optional keyer
connection state
```

Drivers are responsible for translating radio-specific VFO and CAT protocol details into that normalized model.

---

# Open Decision Points

## 1. Should `split` live inside `TransmitterState` or at the top level?

Current design places it here:

```rust
pub struct TransmitterState {
    pub split: Option<bool>,
}
```

Reason: split only makes sense if a transmitter exists.

Alternative:

```rust
pub struct RadioState {
    pub split: Option<bool>,
}
```

Reason: many operators think of split as a whole-radio operating state.

## 2. Should RIT and XIT have separate offsets?

Current design uses one shared offset:

```rust
pub struct RitXitState {
    pub rit_enabled: Option<bool>,
    pub xit_enabled: Option<bool>,
    pub offset_hz: Option<RitXitOffsetHz>,
}
```

Alternative:

```rust
pub struct RitXitState {
    pub rit_enabled: Option<bool>,
    pub rit_offset_hz: Option<RitXitOffsetHz>,
    pub xit_enabled: Option<bool>,
    pub xit_offset_hz: Option<RitXitOffsetHz>,
}
```

Some radios may track RIT and XIT offsets independently.

## 3. Should receiver RF/DSP fields be grouped further?

Current design:

```rust
pub struct ReceiverState {
    pub filter: ReceiverFilterState,
    pub rf: ReceiverRfState,
}
```

Alternative:

```rust
pub struct ReceiverState {
    pub filter: ReceiverFilterState,
    pub rf: ReceiverRfState,
    pub dsp: ReceiverDspState,
}
```

This would separate RF hardware controls from DSP/audio controls.

## 4. Should levels be generic `u8` or typed?

Current design uses:

```rust
pub struct LeveledSetting {
    pub enabled: Option<bool>,
    pub level: Option<u8>,
}
```

Alternative examples:

```rust
pub struct AttenuatorDb(pub u8);
pub struct PreampStage(pub u8);
pub struct NoiseReductionLevel(pub u8);
```

Typed levels are more precise but add complexity.

## 5. Should command completion be configurable in v1?

Possible options:

```rust
pub enum CommandCompletion {
    Sent,
    Accepted,
    Observed,
}
```

Question: should v1 expose this, or should v1 pick one default behavior and add configurability later?

## 6. Should `connect` spawn automatically?

Current recommendation:

```rust
Radio::connect(config).await?       // convenience, auto-spawn
Radio::build(config).await?         // advanced, manual task placement
```

Question: should v1 include both, or only the explicit manual model first?

## 7. Should stale/fresh metadata be part of every field?

Current simplified design uses:

```rust
Option<T>
```

Alternative:

```rust
pub struct Known<T> {
    pub value: T,
    pub updated_at: Instant,
    pub source: UpdateSource,
}
```

This would make every field more informative but heavier.

## 8. Should filter bandwidth and shift be grouped or flat?

Current design groups them:

```rust
pub struct ReceiverState {
    pub filter: ReceiverFilterState,
}
```

Alternative:

```rust
pub struct ReceiverState {
    pub filter_bandwidth_hz: Option<u16>,
    pub filter_shift_hz: Option<u16>,
}
```

Grouping is cleaner as more filter-related fields are added, but flat fields are simpler to consume.
