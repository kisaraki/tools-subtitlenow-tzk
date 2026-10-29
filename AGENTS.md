# AGENTS.md

## 1. Purpose

This repository contains a Windows desktop application written primarily in Rust.

The product captures audio from a user-selected Windows application, recognizes speech or media content, translates supported languages into Taiwan Traditional Chinese, and displays synchronized subtitles.

The authoritative product specification is:

```text
SPEC.md
```

Treat `SPEC.md` as the source of truth for product behavior.

This file defines how Codex must work inside this repository.

---

## 2. Project Identity

The product is:

```text
Windows desktop software
Rust
Windows Core Audio
Application / Process audio capture
Streaming speech recognition
AI translation
Subtitle synchronization
Content identification
```

The product is NOT:

```text
a browser extension
a video downloader
a media player
a recording application
a subtitle database
a persistent transcription archive
a DRM bypass tool
```

Do not redesign the project into one of those products.

---

## 3. First Action on Every Task

Before modifying code:

1. Read this `AGENTS.md`.
2. Read the relevant sections of `SPEC.md`.
3. Inspect repository structure and current diff.
4. Inspect existing implementation and tests.
5. Identify the smallest coherent implementation.
6. Stay within the current Phase unless a small prerequisite is unavoidable.

Do not begin by replacing existing architecture based only on assumptions.

---

## 4. Scope Discipline

Implement only the requested Phase or coherent feature.

Do not silently implement future phases.

Do not add unrelated features because they seem useful.

Current major non-goals include:

```text
browser extensions
custom Windows audio driver
DRM circumvention
macOS
Linux
mobile
arbitrary translation targets
permanent subtitle archive
audio recorder
cloud account synchronization
```

---

## 5. Supported Languages

Source languages are fixed:

```text
English
Japanese
French
German
Korean
```

Source selection:

```text
Auto
English
Japanese
French
German
Korean
```

Auto detection must only select among those five.

Target language is fixed:

```text
Taiwan Traditional Chinese
zh-TW
```

Do not add arbitrary target languages.

---

## 6. Taiwan Localization

All user-visible Chinese translation must use natural Taiwan Traditional Chinese.

Do not treat character conversion alone as sufficient localization.

When network subtitle or lyrics data is Simplified Chinese:

```text
Simplified Chinese
→ Traditional character conversion
→ Taiwan terminology localization
→ zh-TW
```

Avoid Mainland-oriented terms when Taiwan equivalents are appropriate.

---

## 7. Translation Quality

Required:

```text
Fast
Standard
High
```

Default:

```text
Standard
```

These modes must affect actual processing strategy.

Do not implement them as cosmetic labels.

---

## 8. Display Modes

Required:

```text
SideChat
BottomSubtitle
Dual
```

Default:

```text
SideChat
```

All views consume one shared `TranscriptTimeline`.

Never duplicate STT or translation pipelines for different views.

---

## 9. Architecture Boundaries

Keep these subsystems independent:

```text
Windows integration
Audio session discovery
Audio capture
Audio preprocessing
Media input
Content resolver
Speech recognition
Translation
Subtitle stabilization
Alignment
Session state
UI
Provider integrations
```

Avoid:

```text
WASAPI → vendor-specific AI client
AI provider → Win32 UI
UI → raw audio callback
resolver → renderer
```

Use domain models and traits.

---

## 10. Audio Source Model

Do not assume:

```text
HWND == Audio Source
```

Internally think in terms of:

```text
Window
→ Process
→ Process Tree
→ Audio Session
→ Windows Audio Engine
```

Browsers are ordinary Windows audio sources.

Do not introduce browser extensions.

---

## 11. Realtime Safety

Audio capture is realtime-sensitive.

The capture thread must never wait for:

```text
network
AI
translation
content lookup
disk
UI
long locks
provider rate limiting
```

Keep capture logic near:

```text
receive audio
timestamp
push/move to bounded buffer
return
```

---

## 12. Concurrency

This project intentionally uses:

```text
multi-threading
asynchronous I/O
pipeline parallelism
```

The goal is not maximum thread count.

The goals are:

```text
low latency
stable capture
responsive UI
multicore utilization
bounded resources
```

---

## 13. Pipeline Parallelism

Allow:

```text
Segment A → Translation
Segment B → STT
Segment C → Preprocess
Segment D → Capture
```

simultaneously.

Do not serialize the entire realtime pipeline.

---

## 14. Thread Model

Prefer approximately:

```text
UI / Win32 message loop → main thread
WASAPI capture          → dedicated thread
Audio preprocessing     → bounded worker
CPU-heavy resolver      → bounded worker pool
STT / translation       → async runtime
Subtitle state          → lightweight worker/task
```

Do not create one OS thread per audio block.

---

## 15. Tokio

Use Tokio for:

```text
network
WebSocket
streaming API
timers
orchestration
cancellation
```

Do not run long CPU-heavy work directly on async executor workers.

Use a bounded worker pool or `spawn_blocking` where appropriate.

---

## 16. Worker Count

Do not hard-code arbitrary CPU worker counts.

Use:

```rust
std::thread::available_parallelism()
```

as an input.

Avoid oversubscription.

---

## 17. Bounded Queues

All high-frequency pipelines must use bounded queues or ring buffers.

Never introduce unbounded audio queues.

For realtime work, bounded latency is more important than preserving stale work forever.

---

## 18. Backpressure

Every producer/consumer boundary must define overload behavior.

If processing falls behind:

```text
keep memory bounded
keep latency bounded
cancel stale work
drop obsolete partial work when necessary
```

Do not accumulate 20–30 seconds of subtitle delay.

---

## 19. Ordering

Every time-sensitive unit must carry:

```text
session id
sequence
timestamp
```

Parallel completion may be out of order.

Never assume completion order equals playback order.

---

## 20. Session Isolation

Every Live or Prepared run must have a unique Session ID.

Old results from a previous session must never appear in a new session.

Use generation/version filtering when appropriate.

---

## 21. Cancellation

Long-running work must be cancellable.

Cancel when:

```text
source changes
session stops
user cancels
application exits
provider restarts
better verified subtitle source supersedes Live AI
Prepared Mode is cancelled
```

Do not leave orphan tasks.

---

## 22. Shared State

Prefer:

```text
message passing
immutable snapshots
small synchronized state
```

Avoid:

```rust
Arc<Mutex<Everything>>
```

Never block audio capture on a global mutex.

Avoid holding synchronous locks across `.await`.

---

## 23. Streaming STT

Prefer streaming recognition.

Avoid:

```text
record whole sentence
upload
wait
translate
display
```

Prefer:

```text
audio stream
→ partial STT
→ partial translation
→ tentative subtitle

final STT
→ final translation
→ stable subtitle
```

---

## 24. Stable / Tentative

Support:

```text
Tentative
Stable
Final
```

and:

```text
stable prefix
+
mutable tail
```

Minimize visible flicker.

Allow rollback of uncommitted translation text.

---

## 25. Translation Context

Standard and High quality may use:

```text
previous stable sentences
current sentence
content metadata
session glossary
speaker context if available
```

Do not translate every sentence in isolation.

---

## 26. Session Glossary

Temporary glossary is allowed for:

```text
character names
place names
technical terms
organizations
media-specific terminology
```

It must be:

```text
RAM only
session only
```

---

## 27. AI Provider Design

Do not hard-code core logic around one AI vendor.

Use traits/interfaces such as:

```rust
trait SpeechRecognizer: Send + Sync { ... }
trait Translator: Send + Sync { ... }
trait ContentIdentifier: Send + Sync { ... }
```

Provider-specific request/response structures stay inside provider modules.

---

## 28. Provider Capabilities

Represent capabilities explicitly.

Possible examples:

```text
streaming STT
partial transcript
timestamps
word timestamps
language detection
cancellation
streaming translation
```

Do not scatter provider-name conditionals across the app.

---

## 29. Persistent Connections

Prefer provider-supported persistent streaming connections.

Do not open a new TLS connection for every small audio chunk.

---

## 30. API Concurrency

Never issue unlimited simultaneous cloud requests.

Use:

```text
Semaphore
bounded request queue
rate limiter
```

Account for:

```text
provider limits
latency
cost
network bandwidth
ordering
```

---

## 31. Retry

Classify errors as retryable or non-retryable.

Use bounded exponential backoff with jitter where appropriate.

Realtime retries must also consider staleness.

---

## 32. Provider Failover

Do not silently move private audio to a different provider.

Cross-provider fallback requires an explicit future product decision.

---

## 33. Content Resolver

Support:

```text
Unknown
Speech
Music
MovieOrShow
Mixed
```

Resolver work should run in parallel with Live STT.

Never make Live STT wait for song identification.

---

## 34. Subtitle Sources

Support conceptual sources:

```text
LiveAi
ExistingSubtitle
ExistingLyrics
PreparedTranslation
Hybrid
```

All candidates go through a single source arbiter.

Do not let individual resolver modules write directly to UI.

---

## 35. Source Arbitration

Consider:

```text
confidence
alignment
freshness
availability
translation quality
```

Do not switch away from Live AI merely because a weak candidate exists.

---

## 36. Existing Subtitle Preference

General preference:

```text
verified Prepared text
native zh-TW
other Traditional Chinese localized for Taiwan
Simplified Chinese localized to zh-TW
source-language subtitle translated to zh-TW
Live AI
```

Alignment and confidence still matter.

---

## 37. Hybrid Mode

Hybrid may combine:

```text
existing subtitle
+
Live AI gap filling
```

Use one unified `TranscriptTimeline`.

---

## 38. Music Recognition

Use sliding / repeated identification.

Support:

```text
candidate voting
confidence threshold
hysteresis
transition state
```

Do not force an incorrect match during medleys or crossfades.

---

## 39. Movie / Show Identification

Do not rely on one universal audio fingerprint database.

Use multiple available signals:

```text
window title
process
filename
ASR dialogue
language
duration
subtitle metadata
remote metadata
```

Only adopt existing subtitles after strong confidence and alignment.

---

## 40. Prepared Mode

Prepared Mode accepts:

```text
local media files
supported HTTP(S) URLs
```

Prefer:

```text
native zh-TW
other Traditional Chinese
Simplified Chinese → zh-TW
source subtitle → translation
ASR → translation
```

Do not perform unnecessary ASR.

---

## 41. Prepared Parallelism

Prepared Mode may use more aggressive bounded multicore processing.

Independent segments may run concurrently when context is preserved.

Always:

```text
bound concurrency
preserve timestamps
restore output order
respect rate limits
perform merge / consistency validation
```

---

## 42. Prepared Cancellation

Cancellation must:

```text
stop scheduling new work
cancel remote requests where possible
allow workers to stop safely
release session memory
remove temp files
return to safe state
```

---

## 43. Prepared Playback

When processing is ready:

```text
notify the user
do not auto-play
```

On playback, re-align to actual audio.

Support seek / pause / restart without relying only on wall-clock time.

---

## 44. No Permanent Audio Storage

Hard rule.

Do not implement:

```text
Save Audio
recording history
Export WAV
Export PCM
persistent audio cache
persistent capture files
```

Audio buffers must be:

```text
bounded
in-memory
short-lived
session-scoped
```

---

## 45. No Persistent Content Cache

Do not persist:

```text
audio
ASR transcripts
translations
lyrics
subtitles
fingerprints
media identification
alignment
remote metadata
```

Do not reintroduce disk caching as an optimization.

---

## 46. Remote Subtitle / Lyrics Data

Network-obtained subtitles, lyrics, translations, transcripts, timelines, and metadata are session-only.

They may live in RAM for the active session.

Do not persist them across app launches.

---

## 47. Temporary Files

Use OS temp only when technically required.

Rules:

```text
app-owned unique path
delete after use
delete on cancellation
delete on recoverable error
safe startup cleanup of stale app temp
never treat temp as cache
```

---

## 48. No Permanent Subtitle Export

Unless SPEC.md changes, the first version does not provide:

```text
Export SRT
Export VTT
Save translated subtitle archive
```

Prepared translation exists for playback assistance only.

---

## 49. Privacy

Always make the active capture source visible.

Never silently capture another source.

Do not permanently save captured media content.

Make cloud processing state understandable to the user.

---

## 50. Telemetry

Default:

```text
No telemetry
```

Do not add analytics without explicit product approval.

Never send audio/transcripts/URLs/source app names for analytics.

---

## 51. Secrets

Never hard-code:

```text
API keys
passwords
access tokens
private certificates
```

Prefer Windows secure credential storage.

Do not place secrets in normal plaintext config or logs.

---

## 52. Network Security

Use validated secure transport:

```text
HTTPS
WSS
provider-supported TLS transport
```

Do not disable certificate verification in production.

---

## 53. Logging

Use structured logging when practical.

Levels:

```text
ERROR
WARN
INFO
DEBUG
TRACE
```

Never log:

```text
API keys
Authorization headers
passwords
raw PCM
signed URL credentials
full private transcript by default
```

---

## 54. Error Handling

Use typed errors.

Avoid `unwrap()` / `expect()` for recoverable runtime conditions.

Handle:

```text
audio source disappearance
audio device change
network disconnect
AI timeout
rate limits
invalid credential
malformed provider response
unsupported media
subtitle mismatch
```

---

## 55. unsafe Rust

Keep unsafe code:

```text
small
isolated
reviewable
```

Important unsafe blocks require:

```rust
// SAFETY:
```

with reasoning about pointer validity, buffer bounds, ownership, lifetime, COM lifetime, and thread requirements.

Do not use unsafe just to silence the borrow checker.

---

## 56. COM / Win32 Resources

Respect COM apartment requirements.

Use RAII for handles/resources where possible.

Do not leak handles during repeated source switching.

Do not move COM interfaces across threads unless permitted.

---

## 57. Configuration

Persistent preferences are allowed:

```text
window position
font
font size
opacity
display mode
translation quality
source-language preference
monitor preference
provider selection
```

Persistent config must NOT contain:

```text
transcript
subtitle
lyrics
translation output
fingerprint
audio
```

Version the configuration schema.

---

## 58. UI

Default mode:

```text
SideChat
```

Required modes:

```text
SideChat
BottomSubtitle
Dual
```

Switching display mode must not restart backend processing.

---

## 59. SideChat

Support:

```text
source text
zh-TW translation
timestamp
bounded display history for this app launch
auto-follow
manual scrollback
search
return-to-live
```

Tentative text should update one current item rather than create a new bubble per token.

History is RAM-only and bounded.

2026-10-25 explicit user decision: retain bounded display-only subtitle history for this app launch across backend stop, error, source switch and reconnect; only the user's Clear subtitles action empties it (oldest-row eviction at the existing bound and app-exit RAM release remain). Keep full processing identities on rows, label historical content, reject late old-session updates, and never feed retained display history back into recognition, translation, alignment or source arbitration. Backend cancellation and session-only work cleanup remain mandatory. See SPEC.md and ADR 0095.

---

## 60. BottomSubtitle

Support:

```text
always-on-top
position adjustment
lock
click-through
font
font size
outline
shadow
background opacity
monitor selection
original + translation
translation-only
```

Default should prioritize zh-TW.

---

## 61. Multi-monitor / DPI

UI must be per-monitor DPI aware.

Test common scaling levels.

Support monitor selection and, where practical, follow-source-window behavior.

Do not add invasive hooks to support exclusive fullscreen.

---

## 62. Application State

Prefer an explicit state machine instead of many booleans.

Conceptual states:

```text
Idle
SourceSelected
StartingLive
LiveRunning
PreparingMedia
PreparedReady
PreparedPlaying
Stopping
Recovering
Error
```

---

## 63. Status Visibility

UI should expose states such as:

```text
Idle
Listening
Recognizing
Translating
Using existing subtitle
Preparing media
Reconnecting
Stopped
Error
```

---

## 64. Performance Instrumentation

Debug mode should be able to measure:

```text
audio queue depth
STT queue depth
translation queue depth
capture latency
STT first-partial
STT final
translation first-partial
translation final
UI latency
dropped frames
dropped stale segments
worker count
active requests
reconnect count
```

Prefer P50 / P95 / maximum when practical.

---

## 65. Performance Targets

Engineering targets, not guaranteed SLA:

```text
first useful subtitle < 1 second
desired: 300–700 ms
translation ready → visible UI < 100 ms
```

Never falsify benchmarks.

---

## 66. Testing

Core logic must be testable without live cloud services.

Provide mocks where appropriate:

```text
MockSpeechRecognizer
MockTranslator
MockContentResolver
MockMediaSource
```

Test:

```text
out-of-order completion
source switch
cancellation
slow consumer
queue saturation
late old-session result
provider failure
temporary-file cleanup
zh-CN → zh-TW localization
```

---

## 67. Manual Test Matrix

As relevant:

```text
VLC
PotPlayer
Chrome
Edge
Spotify
Windows Media Player / Media Player
at least one game
```

Do not claim compatibility until tested.

---

## 68. Build / Quality Commands

Run when applicable:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
```

For release-sensitive work:

```bash
cargo build --release
```

Never claim a command passed unless actually executed.

---

## 69. Documentation

When behavior/configuration/architecture changes, inspect:

```text
SPEC.md
README.md
AGENTS.md
docs/
```

Do not duplicate the entire SPEC into AGENTS.md.

---

## 70. ADR

For major architectural decisions, use `docs/adr/` if adopted.

Examples:

```text
Process Loopback strategy
No persistent media cache
Single TranscriptTimeline
Session-only remote subtitle policy
SideChat default UI
AI provider abstraction
```

Record WHY a decision exists.

---

## 71. Git Hygiene

Never commit:

```text
API keys
passwords
tokens
private certificates
captured audio
private transcripts
remote subtitle caches
translation caches
build output
temporary media
```

Keep `.gitignore` current.

Prefer focused commits.

Do not discard unrelated user changes.

Avoid destructive commands unless explicitly requested and appropriate.

---

## 72. Model / Reasoning Effort

Use the lowest effort that can safely complete the task.

Suggested policy:

```text
Low
→ docs
→ formatting
→ simple rename
→ trivial isolated test

Medium
→ normal Rust implementation
→ UI work
→ provider integration
→ moderate refactor

High
→ WASAPI
→ COM lifetime
→ unsafe Rust
→ concurrency
→ cancellation
→ pipeline architecture
→ subtitle alignment
→ source arbitration
→ security-sensitive code
→ difficult bugs
```

If the environment cannot switch effort automatically, do not pretend it did.

Correctness and safety take priority over token savings.

---

## 73. Token Efficiency

Prefer:

```text
targeted file reads
repository search
small coherent patches
reuse existing abstractions
focused tests during iteration
full required validation before completion
```

Do not skip:

```text
security review
unsafe review
concurrency review
tests
```

merely to save tokens.

---

## 74. No Fake Implementation

Do not implement placeholder behavior that appears real.

Avoid:

```text
random confidence values
fake audio sources
hard-coded subtitles presented as recognition
always-success provider calls
silent dummy fallback
```

Mocks are allowed only in explicit test/development contexts.

---

## 75. No Silent Degradation

If a feature cannot operate as requested:

```text
return meaningful error
show appropriate state
document limitation
```

Do not silently switch to a materially different privacy or capture mode.

---

## 76. User-visible Chinese

Application UI Chinese must use Taiwan Traditional Chinese and Taiwan terminology.

Technical source identifiers/comments may use English.

---

## 77. Magic Numbers

Name and document:

```text
latency thresholds
queue sizes
retry intervals
confidence thresholds
sliding-window lengths
stale timeouts
```

Do not scatter unexplained constants through business logic.

---

## 78. Phase Discipline

Stay within the current Phase.

Do not silently implement future phases.

Do not create fake production implementations merely to satisfy interfaces.

Use explicit mocks/stubs when appropriate.

---

## 79. Definition of Done

A task is complete only when all applicable conditions are satisfied:

```text
[ ] Behavior matches SPEC.md
[ ] Code builds
[ ] Formatting passes
[ ] Clippy passes without hidden warnings
[ ] Relevant tests pass
[ ] Errors are handled
[ ] Audio capture remains non-blocking
[ ] Queues are bounded
[ ] Concurrency is bounded
[ ] Cancellation is correct
[ ] Session isolation is correct
[ ] No thread/task/resource leaks
[ ] No hard-coded secrets
[ ] No persistent audio
[ ] No persistent subtitle/translation/lyrics cache
[ ] Remote subtitle data remains session-only
[ ] Simplified Chinese is localized to Taiwan zh-TW
[ ] unsafe code is justified
[ ] Documentation is updated
[ ] Known limitations are reported
[ ] No untested behavior is falsely claimed as tested
```

---

## 80. Standard Codex Workflow

```text
Read AGENTS.md
↓
Read relevant SPEC.md
↓
Inspect repository / current diff
↓
Identify affected boundaries
↓
Plan smallest coherent change
↓
Implement
↓
Run targeted tests
↓
Review errors / concurrency / privacy
↓
Run final validation
↓
Update documentation
↓
Report completed work and limitations
```

Do not skip repository inspection.

---

## 81. Final Engineering Principles

Preserve these rules:

```text
Never block audio capture.
Keep latency bounded.
Keep memory bounded.
Keep concurrency bounded.
Never persist captured audio.
Never create persistent subtitle / translation / lyrics caches.
Remote subtitles are session-only.
Never silently transmit private content to an unconfigured provider.
Never allow old-session results into a new session.
Always output Taiwan Traditional Chinese.
Prefer verified existing text when correctly identified and aligned.
Fall back gracefully to Live AI.
Use one TranscriptTimeline.
Use multiple renderers, not multiple translation pipelines.
Make cancellation explicit.
Make session identity explicit.
Measure before optimizing.
Do not hide incomplete work.
```
