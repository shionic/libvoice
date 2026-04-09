# libvoice-web-demo

Browser demo for `libvoice-wasm`.

The app:

- requests microphone access
- captures mono audio in 100ms chunks
- streams each chunk into `VoiceAnalyzerStream`
- smooths per-frame values inside each 100ms interval for the "Moment Data" panel
- shows the latest chunk summary separately in the "Summarize Data" panel
- prints the final `analyzer.finalize()` result as raw JSON after stop

## Run

Build the wasm package into the demo directory:

```bash
cd libvoice-web-demo
npm run build
```

This writes the generated web package into:

```bash
./pkg
```

Serve the static files:

```bash
cd libvoice-web-demo
npm run serve
```

Open <http://localhost:4173>.

## Notes

- `wasm-pack` must be installed locally.
- The browser must support `AudioWorklet`.
- The final JSON output is not reshaped or reduced. It is the direct result of `VoiceAnalyzerStream.finalize()`.
- `npm run build:wasm` remains available as an alias for the same `wasm-pack` command.
