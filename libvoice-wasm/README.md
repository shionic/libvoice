# libvoice-wasm

`libvoice-wasm` exposes `libvoice` for browser use through `wasm-bindgen`.

The wrapper expects decoded mono PCM samples as `Float32Array`-compatible data. Audio decoding stays in JavaScript, Web Audio, or another browser-side pipeline.

## Build

With `wasm-pack`:

```bash
wasm-pack build libvoice-wasm --target web
```

With `cargo` only:

```bash
cargo build -p libvoice-wasm --target wasm32-unknown-unknown --release
```

## JavaScript API

### One-shot analysis

```js
import init, { analyzeMonoF32 } from "./pkg/libvoice_wasm.js";

await init();

const samples = new Float32Array([...]);
const result = analyzeMonoF32(16000, samples, {
  frameSize: 2048,
  hopSize: 512,
}, true);

console.log(result.report.overall);
console.log(result.frames);
```

Arguments:

- `sampleRate: number`
- `samples: Float32Array | number[]`
- `options?: AnalyzerOptions`
- `includeFrames: boolean`

If `includeFrames` is `false`, the function returns an `AnalysisReport`.

If `includeFrames` is `true`, the function returns:

```ts
{
  report: AnalysisReport;
  frames: FrameAnalysis[];
}
```

### Incremental analysis

```js
import init, { VoiceAnalyzerStream } from "./pkg/libvoice_wasm.js";

await init();

const analyzer = new VoiceAnalyzerStream(16000, {
  frameSize: 2048,
  hopSize: 512,
});

const chunkResult = analyzer.processChunk(new Float32Array([...]));
console.log(chunkResult.chunk);
console.log(chunkResult.frames);

const overall = analyzer.finalize();
console.log(overall);
```

## Analyzer Options

The options object uses camelCase keys:

- `frameSize`
- `hopSize`
- `minPitchHz`
- `maxPitchHz`
- `pitchClarityThreshold`
- `rolloffRatio`
- `voicedRmsThreshold`
- `voicedMaxSpectralFlatness`
- `voicedMaxZeroCrossingRate`

## Notes

- Input is expected to be mono PCM normalized to roughly `[-1.0, 1.0]`.
- Multichannel folding and compressed audio decoding should be handled in JS before passing samples into the WASM module.
- The wrapper returns plain JS objects through `serde-wasm-bindgen`, not JSON strings.
