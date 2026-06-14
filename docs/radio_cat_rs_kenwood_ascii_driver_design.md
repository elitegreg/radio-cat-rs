# radio-cat-rs Kenwood-Style ASCII Driver Design

## Purpose

This document describes a proposed backend driver architecture for `radio-cat-rs`, focused on sharing one Kenwood-style ASCII CAT protocol implementation across:

- Kenwood radios
- Elecraft radios
- Newer Yaesu radios that use Kenwood-like ASCII CAT commands

The core idea is that these radios can share the same protocol engine because they use a broadly similar ASCII command style, usually with short command prefixes and semicolon-terminated frames. They differ in command availability, argument formatting, response shapes, mode values, VFO handling, and radio-specific extensions.

The implementation should therefore share protocol handling and common semantic command families while isolating model-specific behavior in dialects/codecs.

---

## Design Summary

The proposed stack is:

```text
radio-cat-rs public async API
        ↓
radio actor / state machine / polling-or-native-events
        ↓
KenwoodAsciiDriver<Dialect>
        ↓
shared semicolon ASCII CAT protocol engine
        ↓
dialect-specific command codecs
        ↓
serial / TCP transport
```

The shared driver owns the mechanics of CAT communication:

- transport reads/writes
- frame splitting
- semicolon-terminated ASCII parsing
- transaction queueing
- polling
- timeout/retry behavior
- unsolicited message handling
- state updates
- changed-flag generation
- async event emission

The dialect owns radio-specific behavior:

- which commands are supported
- which commands should be sent
- how command arguments are encoded
- how responses are decoded
- mode mappings
- frequency formatting
- VFO/split model
- poll plan
- quirks and extensions

---

## Key Principle

The rest of `radio-cat-rs` should not know whether a radio is Kenwood, Elecraft, or Yaesu at the CAT-command level.

Everything above the driver should operate on normalized types:

```text
RadioCommand
RadioEvent
RadioState
RadioCapabilities
ChangedFlags
```

Only the protocol/dialect layer should know about raw commands such as:

```text
FA;
FA00014074000;
MD;
MD2;
IF;
TX;
RX;
```

---

## Naming

Avoid naming the shared implementation simply `KenwoodDriver`, because it will be used for more than Kenwood radios.

Recommended names:

- `KenwoodAscii`
- `AsciiCat`
- `SemicolonCat`
- `KenwoodStyleAscii`

A good compromise is:

```text
protocol::kenwood_ascii
```

This preserves the common ham-radio naming while making it clear that this is a protocol family, not a single radio model.

---

## What Is Shared

### Shared protocol mechanics

The following should be implemented once:

- ASCII command frame type
- semicolon frame splitter
- read loop
- write loop
- transaction queue
- response timeout logic
- command serialization order
- incoming frame dispatch
- unknown frame logging
- polling scheduler
- connection state machine
- state reducer integration

### Shared semantic command families

Many radios share the same conceptual commands, even if the exact argument formats differ.

Examples:

| Semantic operation | Common CAT family |
|---|---|
| Query VFO A frequency | `FA;` |
| Set VFO A frequency | `FA...;` |
| Query VFO B frequency | `FB;` |
| Set VFO B frequency | `FB...;` |
| Query/set mode | `MD` family |
| Query status | `IF;` or similar |
| Set PTT transmit | `TX;`, `TX0;`, `TX1;`, etc. |
| Return to receive | `RX;` |
| Query/set split/VFO routing | `FR`, `FT`, `SP`, or radio-specific equivalents |
| Query radio identity | `ID;`, model-specific alternatives |

The semantic operation should be shared. The raw argument codec should be dialect-specific.

---

## What Is Dialect-Specific

The following should be owned by the dialect/model layer:

- exact command support
- exact argument width
- exact argument units
- mode enum mappings
- VFO addressing model
- status block parsing
- split handling
- PTT command shape
- RIT/XIT command shape
- data-mode representation
- radio-specific commands
- firmware quirks
- whether commands are safe to send
- preferred polling plan

For example, frequency commands may all be conceptually `FA`/`FB`, but the argument formatting can differ:

```text
Kenwood TS-590 style:
  FA00014195000;    11-digit Hz-style value

Elecraft K4 style:
  FA7100;           may accept compact kHz-style input
  FA00007100000;    may report 11-digit Hz-style value

Yaesu FT-DX10 style:
  FA014250000;      Yaesu-style width/format
```

The driver should expose one semantic command:

```rust
RadioCommand::SetFrequency {
    vfo: Vfo::A,
    hz: 14_195_000,
}
```

The dialect should decide how to encode it for the specific radio.

---

## Sending Rule

Sending must be capability-gated and dialect-encoded.

The shared driver should not blindly send a raw Kenwood command just because the semantic operation exists.

Correct flow:

```text
RadioCommand
  ↓
check capability / dialect support
  ↓
dialect-specific encode
  ↓
shared ASCII frame writer
  ↓
transport
```

If a command is unsupported for a radio, return a structured unsupported-command error.

Example:

```rust
pub enum DriverError {
    UnsupportedCommand {
        command: &'static str,
        dialect: DialectId,
    },
    EncodeError(String),
    DecodeError(String),
    Timeout,
    TransportError(String),
}
```

---

## Receiving Rule

Receiving should be tolerant, but semantic decoding should still be dialect-aware.

Raw protocol frame handling can be shared:

```text
incoming bytes
  ↓
semicolon frame splitter
  ↓
AsciiFrame
```

But converting a frame into meaning should use the selected dialect:

```text
AsciiFrame
  ↓
dialect decoder
  ↓
RadioEvent
  ↓
state reducer
  ↓
ChangedFlags / async update
```

This is important because the same prefix can have different argument shapes or meanings between radios.

Examples:

```text
MD;       common mode query shape
MD1;      possible Kenwood/Elecraft response or set shape
MD01;     possible Yaesu-style shape

IF;       common status query name
IF...;    response shape varies significantly by radio
```

Unknown received frames should not be fatal. They should usually be logged and ignored unless they indicate a known error condition.

Recommended behavior:

```rust
match dialect.decode(&frame)? {
    Some(event) => state_reducer.apply(event),
    None => log::debug!("unknown or ignored CAT frame: {:?}", frame),
}
```

---

## Capabilities

Capabilities should control what the app/API exposes and what the driver is allowed to send.

Example:

```rust
pub struct RadioCapabilities {
    pub frequency: FrequencyCapabilities,
    pub mode: ModeCapabilities,
    pub ptt: PttCapabilities,
    pub vfo: VfoCapabilities,
    pub split: SplitCapabilities,
    pub rit: Option<RitCapabilities>,
    pub xit: Option<XitCapabilities>,
    pub cw: Option<CwCapabilities>,
    pub polling: PollingCapabilities,
}
```

Capabilities answer questions like:

- Can this radio set VFO A frequency?
- Can this radio set VFO B frequency?
- Can this radio report mode?
- Can this radio report PTT state?
- Can this radio key PTT over CAT?
- Can this radio send CW over CAT?
- Does this radio support split control?
- Does this radio provide native/unsolicited async updates?
- Which fields must be polled?

Capabilities should not be the primary parsing mechanism. Parsing should be tolerant, because radios may emit unexpected frames due to auto-info mode, front-panel changes, firmware differences, or other software.

---

## Normalized Public Commands

The app-facing command model should stay semantic and radio-neutral.

Example:

```rust
pub enum RadioCommand {
    QueryFrequency { vfo: Vfo },
    SetFrequency { vfo: Vfo, hz: u64 },

    QueryMode { target: VfoTarget },
    SetMode { target: VfoTarget, mode: RadioMode },

    QueryStatus,

    SetPtt(bool),

    QuerySplit,
    SetSplit(bool),

    QueryRit,
    SetRitHz(i32),
    ClearRit,

    QueryXit,
    SetXitHz(i32),
    ClearXit,

    SendCw(String),

    RawAscii(String),
}
```

`RawAscii` is useful for diagnostics, but should not be the normal API path.

---

## Normalized Events

All radios should produce the same event stream shape, regardless of whether the information came from:

- native unsolicited CAT messages
- command responses
- periodic polling
- optimistic local updates after a successful set command

Example:

```rust
pub enum RadioEvent {
    Connected,
    Disconnected,

    FrequencyChanged { vfo: Vfo, hz: u64 },
    ModeChanged { target: VfoTarget, mode: RadioMode },
    PttChanged(bool),
    SplitChanged(bool),

    RitChanged { hz: i32 },
    XitChanged { hz: i32 },

    StatusSnapshot(RadioSnapshot),

    UnknownFrame(String),
    DriverWarning(String),
}
```

---

## State Reducer

All decoded events should flow through a state reducer.

The reducer owns:

- current `RadioState`
- previous `RadioState`
- changed-flag calculation
- async update emission

Example flow:

```text
decoded frame
  ↓
RadioEvent
  ↓
StateReducer::apply(event)
  ↓
RadioState updated
  ↓
ChangedFlags calculated
  ↓
subscribers notified
```

This lets radios with native async behavior and radios with polling behavior look identical to the public API.

---

## Async Updates and Polling

The public API should expose async radio updates.

Some radios may support native async/auto-info messages. Others must simulate async updates by polling and comparing state.

The driver architecture should treat both cases the same after decoding:

```text
native unsolicited frame → RadioEvent → StateReducer
poll response changed     → RadioEvent → StateReducer
```

The app should not need to know which mechanism produced the update.

### Polling plan

Each dialect should define a polling plan.

Example:

```rust
pub struct PollPlan {
    pub fast: Vec<PollItem>,
    pub medium: Vec<PollItem>,
    pub slow: Vec<PollItem>,
}

pub struct PollItem {
    pub command: RadioCommand,
    pub interval: Duration,
    pub priority: PollPriority,
}
```

Possible polling categories:

```text
Fast:
  frequency
  mode
  PTT state

Medium:
  split
  active VFO
  RIT/XIT

Slow:
  filter
  power
  radio-specific status
```

User commands should have higher priority than polling.

Recommended queue priority:

```text
highest: emergency stop / RX / PTT off
high:    user commands, PTT, CW keying
normal:  frequency/mode changes
low:     polling
lowest:  diagnostics/background refresh
```

---

## Core Traits

### Shared driver

```rust
pub struct KenwoodAsciiDriver<D, T>
where
    D: KenwoodAsciiDialect,
    T: CatTransport,
{
    dialect: D,
    transport: T,
    state: RadioState,
    reducer: StateReducer,
    transactions: TransactionQueue,
}
```

### Dialect trait

```rust
pub trait KenwoodAsciiDialect: Send + Sync + 'static {
    fn dialect_id(&self) -> DialectId;
    fn display_name(&self) -> &'static str;

    fn capabilities(&self) -> RadioCapabilities;
    fn poll_plan(&self) -> PollPlan;

    fn encode(&self, command: RadioCommand) -> Result<Option<AsciiFrame>, DriverError>;
    fn decode(&self, frame: &AsciiFrame) -> Result<Option<RadioEvent>, DriverError>;
}
```

### Transport trait

```rust
#[async_trait::async_trait]
pub trait CatTransport: Send {
    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), TransportError>;
    async fn read_some(&mut self, buf: &mut [u8]) -> Result<usize, TransportError>;
    async fn flush(&mut self) -> Result<(), TransportError>;
}
```

Possible transport implementations:

```text
SerialTransport
TcpTransport
MockTransport
RecordingTransport
ReplayTransport
```

---

## Command Codecs

Avoid implementing every command as one giant `match dialect` block.

Prefer shared command-family code with small dialect-specific codec traits.

### Frequency codec

```rust
pub trait FrequencyCodec {
    fn encode_fa_set(&self, hz: u64) -> Result<AsciiFrame, DriverError>;
    fn encode_fb_set(&self, hz: u64) -> Result<AsciiFrame, DriverError>;

    fn encode_fa_query(&self) -> AsciiFrame {
        AsciiFrame::new("FA;".to_string())
    }

    fn encode_fb_query(&self) -> AsciiFrame {
        AsciiFrame::new("FB;".to_string())
    }

    fn decode_fa_response(&self, args: &str) -> Result<u64, DriverError>;
    fn decode_fb_response(&self, args: &str) -> Result<u64, DriverError>;
}
```

### Mode codec

```rust
pub trait ModeCodec {
    fn encode_mode_query(&self, target: VfoTarget) -> Result<AsciiFrame, DriverError>;
    fn encode_mode_set(&self, target: VfoTarget, mode: RadioMode) -> Result<AsciiFrame, DriverError>;
    fn decode_mode_response(&self, frame: &AsciiFrame) -> Result<ModeEvent, DriverError>;
}
```

### PTT codec

```rust
pub trait PttCodec {
    fn encode_set_ptt(&self, ptt: bool) -> Result<AsciiFrame, DriverError>;
    fn encode_query_ptt(&self) -> Option<AsciiFrame>;
    fn decode_ptt_response(&self, frame: &AsciiFrame) -> Result<Option<bool>, DriverError>;
}
```

---

## Dialect Implementations

Initial dialects could include:

```rust
pub struct KenwoodTs590Dialect;
pub struct ElecraftK4Dialect;
pub struct YaesuFtDx10Dialect;
pub struct YaesuFt710Dialect;
pub struct YaesuFt991aDialect;
```

Each dialect should define:

- capabilities
- poll plan
- command encoders
- response decoders
- mode map
- known quirks

Example:

```rust
impl KenwoodAsciiDialect for KenwoodTs590Dialect {
    fn dialect_id(&self) -> DialectId {
        DialectId::KenwoodTs590
    }

    fn display_name(&self) -> &'static str {
        "Kenwood TS-590"
    }

    fn capabilities(&self) -> RadioCapabilities {
        // model-specific capabilities
    }

    fn poll_plan(&self) -> PollPlan {
        // model-specific polling preferences
    }

    fn encode(&self, command: RadioCommand) -> Result<Option<AsciiFrame>, DriverError> {
        // route to command-family codecs
    }

    fn decode(&self, frame: &AsciiFrame) -> Result<Option<RadioEvent>, DriverError> {
        // route by prefix, using TS-590-specific response parsing where needed
    }
}
```

---

## Command Routing

A shared command router can dispatch by command prefix.

Example:

```rust
pub fn command_prefix(frame: &AsciiFrame) -> Option<&str> {
    frame.as_str().get(0..2)
}
```

Then dialect decoding can look like:

```rust
match command_prefix(frame) {
    Some("FA") => self.frequency.decode_fa_response(frame.args()),
    Some("FB") => self.frequency.decode_fb_response(frame.args()),
    Some("MD") => self.mode.decode_mode_response(frame),
    Some("IF") => self.status.decode_if_response(frame),
    Some("TX") | Some("RX") => self.ptt.decode_ptt_response(frame),
    _ => Ok(None),
}
```

The prefix router can be shared, but the handlers should be dialect-aware.

---

## Command Classes

Commands should be classified by expected response behavior.

```rust
pub enum ResponseKind {
    None,
    EchoOnly,
    UntilSemicolon,
    MatchingPrefix(&'static str),
    Custom,
}
```

Examples:

```rust
pub struct OutgoingCatCommand {
    pub frame: AsciiFrame,
    pub response_kind: ResponseKind,
    pub timeout: Duration,
    pub retries: u8,
    pub priority: CommandPriority,
}
```

This matters because some commands may:

- return no response
- echo the command
- return a matching command prefix
- return a status block
- emit additional unsolicited frames
- behave differently based on radio settings

---

## Optimistic Updates

After a successful set command, the driver may optimistically update local state before the next poll confirms it.

Example:

```text
app sends SetFrequency(A, 14_074_000)
  ↓
dialect encodes FA command
  ↓
transport write succeeds
  ↓
state is optimistically updated
  ↓
next poll confirms or corrects the value
```

This makes the UI feel responsive.

However, optimistic updates should be marked as unconfirmed if useful.

Possible state metadata:

```rust
pub struct StateField<T> {
    pub value: Option<T>,
    pub source: StateSource,
    pub updated_at: Instant,
}

pub enum StateSource {
    PollConfirmed,
    NativeEvent,
    CommandResponse,
    Optimistic,
}
```

---

## Error Handling

Errors should be structured and recoverable where possible.

Important categories:

- unsupported command
- invalid argument
- encode failure
- decode failure
- timeout
- transport disconnected
- malformed frame
- unknown frame
- command rejected by radio
- state conflict

Unknown frames should usually not disconnect the radio.

Repeated transport failures should transition the actor into an error or reconnecting state.

---

## Connection State Machine

Each radio actor should maintain a connection state.

```rust
pub enum RadioConnectionState {
    Disconnected,
    Connecting,
    Identifying,
    Ready,
    Error { message: String },
    Reconnecting,
}
```

Recommended startup flow:

```text
open transport
  ↓
flush stale input
  ↓
identify radio if supported
  ↓
load dialect/capabilities
  ↓
perform initial poll
  ↓
enter Ready
  ↓
start normal polling and event processing
```

---

## Actor Model

One actor/task should own each radio connection.

Do not allow multiple parts of the backend to write to the serial/TCP transport directly.

Recommended structure:

```text
API/client code
  ↓
RadioManager
  ↓
RadioActor
  ↓
KenwoodAsciiDriver<Dialect>
  ↓
CatTransport
```

The actor owns:

- command queue
- polling scheduler
- connection lifecycle
- state reducer
- async event broadcast

This avoids shared mutable serial-port access and makes command ordering explicit.

---

## Module Layout

Suggested source layout:

```text
src/
  radio/
    api.rs
    actor.rs
    manager.rs
    state.rs
    event.rs
    command.rs
    capabilities.rs
    changed_flags.rs
    error.rs

  transport/
    mod.rs
    serial.rs
    tcp.rs
    mock.rs
    recording.rs
    replay.rs

  protocol/
    kenwood_ascii/
      mod.rs
      frame.rs
      driver.rs
      dialect.rs
      transaction.rs
      poll.rs
      router.rs
      error.rs

      commands/
        mod.rs
        frequency.rs
        mode.rs
        ptt.rs
        status.rs
        split.rs
        rit.rs
        xit.rs
        cw.rs
        identity.rs

      dialects/
        mod.rs
        kenwood_ts590.rs
        elecraft_k4.rs
        yaesu_ftdx10.rs
        yaesu_ft710.rs
        yaesu_ft991a.rs

    icom_civ/
      mod.rs

    flex/
      mod.rs
```

---

## Testing Strategy

Testing should be built into the design from the start.

### Unit tests

Test each command codec independently:

- encode frequency command
- decode frequency response
- encode mode command
- decode mode response
- parse IF/status response
- reject malformed responses
- reject unsupported modes

### Golden tests

Maintain fixture files with raw CAT frames and expected normalized events.

Example:

```text
tests/fixtures/kenwood_ts590/frequency.txt
tests/fixtures/elecraft_k4/mode.txt
tests/fixtures/yaesu_ftdx10/status.txt
```

### Mock transport tests

Use `MockTransport` to simulate:

- normal responses
- delayed responses
- timeouts
- malformed frames
- unsolicited frames
- interleaved poll/user commands

### Replay tests

A `RecordingTransport` and `ReplayTransport` can capture real radio sessions and replay them in CI.

This will be especially useful when comparing dialect behavior across Kenwood, Elecraft, and Yaesu radios.

---

## Implementation Phases

### Phase 1: Core protocol engine

- `AsciiFrame`
- semicolon splitter
- `CatTransport`
- `MockTransport`
- transaction queue
- basic command send/read

### Phase 2: Normalized state/event model

- `RadioCommand`
- `RadioEvent`
- `RadioState`
- `ChangedFlags`
- `StateReducer`

### Phase 3: First dialect

Implement one concrete dialect first, likely Kenwood TS-590 or Elecraft K4.

Start with:

- identity
- VFO A frequency query/set
- mode query/set
- PTT set
- basic status polling

### Phase 4: Second dialect

Add a second dialect that is similar but not identical.

Good candidates:

- Elecraft K4 if TS-590 was first
- Yaesu FT-DX10 if you want to force the abstraction to handle wider differences

This will validate the codec/dialect split.

### Phase 5: Async update behavior

- polling scheduler
- changed flags
- event broadcaster
- optional native/auto-info frame handling

### Phase 6: Expand command families

Add:

- VFO B
- split
- RIT/XIT
- CW send
- filter/status fields
- data modes
- radio-specific extensions

---

## Design Decisions

### Decision: Share protocol engine

Kenwood, Elecraft, and newer Yaesu radios should share one Kenwood-style ASCII protocol engine.

Reason:

- same broad ASCII/semicolon command style
- many common command prefixes
- duplicated protocol loops would be wasteful
- async/poll behavior can be normalized above dialects

### Decision: Dialect-specific semantic encoding

Commands should be sent only through dialect-specific encoders.

Reason:

- same semantic command may have different raw CAT argument shape
- command support varies by model
- safer than assuming all radios accept the same command syntax

### Decision: Dialect-aware decoding

Incoming raw frames should be decoded by the selected dialect.

Reason:

- same prefix may mean different things
- response widths vary
- status blocks differ
- mode values differ

### Decision: Unknown frames are non-fatal

Unknown frames should usually be logged and ignored.

Reason:

- radios can emit unsolicited frames
- firmware behavior may vary
- front-panel changes may produce unexpected data
- strict parsing would make the system fragile

### Decision: Polling and native async feed the same reducer

Apps should see one async event stream regardless of source.

Reason:

- simpler public API
- consistent frontend behavior
- supports both modern and older radios

---

## Open Questions

These should be resolved as implementation proceeds:

1. What should the crate call the shared protocol family: `kenwood_ascii`, `ascii_cat`, or `semicolon_cat`?
2. Which radio should be the first concrete dialect?
3. Should optimistic state updates be visible as unconfirmed values?
4. How much raw CAT access should the public API expose?
5. Should dialects be selected manually, auto-detected, or both?
6. How should radio-specific extension commands be exposed without polluting the normalized API?
7. How aggressive should default polling be for each radio?
8. Should command codecs be trait-based, table-driven, or a hybrid?

---

## Recommended Initial Direction

Start with:

```text
protocol::kenwood_ascii
```

Implement:

```text
KenwoodAsciiDriver<Dialect>
KenwoodAsciiDialect
AsciiFrame
CatTransport
MockTransport
StateReducer
PollPlan
```

Then create one concrete dialect and keep the first command set small:

```text
identity
frequency A/B
mode
PTT
basic status
```

After that, add a second dialect before expanding the command surface too much. The second dialect will reveal where the shared/dialect boundary is wrong.

The core rule is:

> Share the ASCII CAT machinery and semantic command families, but never assume the raw command arguments are identical across radios.

