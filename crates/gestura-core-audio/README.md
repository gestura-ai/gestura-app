# gestura-core-audio

Audio capture, speech processing, and STT provider abstractions for Gestura.

## What belongs here

- Audio capture and recording pipeline
- Noise cancellation (RNNoise / platform)
- Speech-to-text provider abstraction
- Speech processing and voice activity detection

Keep protocol transports and GUI/CLI concerns out of this crate.

## Modules

- `audio_capture`        Audio input capture and recording
- `noise_cancellation`   RNNoise-based noise cancellation
- `speech`               Speech processing and voice activity detection
- `stt_provider`         STT provider trait and implementations

## Stable import paths

Most code should import through the facade:

- `gestura_core::audio::*`
- `gestura_core::audio_capture::*`
- `gestura_core::speech::*`
- `gestura_core::stt_provider::*`

The facades live in `crates/gestura-core/src/` and re-export this crate.

## Development

```bash
cargo test -p gestura-core-audio
cargo clippy -p gestura-core-audio --all-targets --all-features -- -D warnings
```

