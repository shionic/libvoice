import initWasm, { VoiceAnalyzerStream } from "../pkg/libvoice_wasm.js";

const CHUNK_DURATION_MS = 100;

const elements = {
  startButton: document.querySelector("#start-button"),
  stopButton: document.querySelector("#stop-button"),
  statusText: document.querySelector("#status-text"),
  sampleRateText: document.querySelector("#sample-rate-text"),
  chunkSizeText: document.querySelector("#chunk-size-text"),
  chunkCountText: document.querySelector("#chunk-count-text"),
  momentGrid: document.querySelector("#moment-grid"),
  summaryGrid: document.querySelector("#summary-grid"),
  finalJson: document.querySelector("#final-json"),
};

const state = {
  audioContext: null,
  stream: null,
  sourceNode: null,
  workletNode: null,
  analyzer: null,
  chunkCount: 0,
  running: false,
  wasmReady: false,
};

const metricDefinitions = {
  moment: [
    ["frameCount", "Voiced Frames"],
    ["pitchHz", "Pitch Hz"],
    ["pitchClarity", "Pitch Clarity"],
    ["rms", "RMS"],
    ["energy", "Energy"],
    ["hnrDb", "HNR dB"],
    ["spectralCentroidHz", "Centroid Hz"],
    ["spectralBandwidthHz", "Bandwidth Hz"],
    ["spectralRolloffHz", "Rolloff Hz"],
    ["spectralFlatness", "Flatness"],
    ["spectralTiltDbPerOctave", "Tilt dB/Oct"],
    ["zcr", "ZCR"],
    ["f1Hz", "F1 Hz"],
    ["f2Hz", "F2 Hz"],
    ["f3Hz", "F3 Hz"],
    ["f4Hz", "F4 Hz"],
  ],
  summary: [
    ["frameCount", "Voiced Frames"],
    ["pitchMeanHz", "Pitch Mean Hz"],
    ["pitchMedianHz", "Pitch Median Hz"],
    ["pitchStdHz", "Pitch Std"],
    ["energyMean", "Energy Mean"],
    ["rmsMean", "RMS Mean"],
    ["hnrMeanDb", "HNR Mean dB"],
    ["centroidMeanHz", "Centroid Mean Hz"],
    ["bandwidthMeanHz", "Bandwidth Mean Hz"],
    ["rolloffMeanHz", "Rolloff Mean Hz"],
    ["flatnessMean", "Flatness Mean"],
    ["tiltMeanDbPerOctave", "Tilt Mean dB/Oct"],
    ["zcrMean", "ZCR Mean"],
    ["f1MeanHz", "F1 Mean Hz"],
    ["f2MeanHz", "F2 Mean Hz"],
    ["f3MeanHz", "F3 Mean Hz"],
    ["f4MeanHz", "F4 Mean Hz"],
  ],
};

bootstrap();

function bootstrap() {
  renderMetricGrid(elements.momentGrid, metricDefinitions.moment);
  renderMetricGrid(elements.summaryGrid, metricDefinitions.summary);
  elements.chunkSizeText.textContent = `${CHUNK_DURATION_MS} ms`;
  elements.startButton.addEventListener("click", startAnalysis);
  elements.stopButton.addEventListener("click", stopAnalysis);
}

async function startAnalysis() {
  if (state.running) {
    return;
  }

  try {
    setStatus("Initializing microphone...");
    elements.finalJson.textContent = "Waiting for stop...";
    resetMetricGrid(elements.momentGrid);
    resetMetricGrid(elements.summaryGrid);
    setChunkCount(0);

    if (!state.wasmReady) {
      await initWasm();
      state.wasmReady = true;
    }

    const stream = await navigator.mediaDevices.getUserMedia({
      audio: {
        channelCount: 1,
        echoCancellation: false,
        noiseSuppression: false,
        autoGainControl: false,
      },
    });

    const audioContext = new AudioContext();
    await audioContext.audioWorklet.addModule("./src/audio-capture-worklet.js");
    await audioContext.resume();

    const sourceNode = audioContext.createMediaStreamSource(stream);
    const workletNode = new AudioWorkletNode(audioContext, "mono-chunk-processor", {
      numberOfInputs: 1,
      numberOfOutputs: 0,
      channelCount: 1,
      processorOptions: {
        chunkDurationMs: CHUNK_DURATION_MS,
      },
    });

    const analyzer = new VoiceAnalyzerStream(audioContext.sampleRate, {
      frameSize: 2048,
      hopSize: 512,
    });

    workletNode.port.onmessage = ({ data }) => {
      if (!state.running || !state.analyzer) {
        return;
      }
      processChunk(data);
    };

    sourceNode.connect(workletNode);

    state.audioContext = audioContext;
    state.stream = stream;
    state.sourceNode = sourceNode;
    state.workletNode = workletNode;
    state.analyzer = analyzer;
    state.chunkCount = 0;
    state.running = true;

    elements.sampleRateText.textContent = `${audioContext.sampleRate} Hz`;
    elements.startButton.disabled = true;
    elements.stopButton.disabled = false;
    setStatus("Analyzing");
  } catch (error) {
    console.error(error);
    await teardownAudio(false);
    setStatus(`Failed: ${error.message ?? String(error)}`);
  }
}

function processChunk(float32Chunk) {
  const result = state.analyzer.processChunk(float32Chunk);
  state.chunkCount += 1;
  setChunkCount(state.chunkCount);

  const moment = smoothMomentData(result.frames);
  const summary = summarizeChunkFields(result.chunk);

  updateMetricGrid(elements.momentGrid, moment);
  updateMetricGrid(elements.summaryGrid, summary);
}

async function stopAnalysis() {
  if (!state.running) {
    return;
  }

  setStatus("Stopping...");

  try {
    const overall = state.analyzer?.finalize() ?? null;
    if (overall) {
      elements.finalJson.textContent = JSON.stringify(overall, null, 2);
    } else {
      elements.finalJson.textContent = "null";
    }
  } catch (error) {
    console.error(error);
    elements.finalJson.textContent = `Failed to finalize: ${
      error.message ?? String(error)
    }`;
  } finally {
    if (state.analyzer) {
      state.analyzer.free();
      state.analyzer = null;
    }

    await teardownAudio(true);
    setStatus("Stopped");
  }
}

async function teardownAudio(keepCounters) {
  state.running = false;

  if (state.workletNode) {
    state.workletNode.port.onmessage = null;
    state.workletNode.disconnect();
    state.workletNode = null;
  }

  if (state.sourceNode) {
    state.sourceNode.disconnect();
    state.sourceNode = null;
  }

  if (state.stream) {
    for (const track of state.stream.getTracks()) {
      track.stop();
    }
    state.stream = null;
  }

  if (state.audioContext) {
    await state.audioContext.close();
    state.audioContext = null;
  }

  elements.startButton.disabled = false;
  elements.stopButton.disabled = true;
  elements.sampleRateText.textContent = "-";

  if (!keepCounters) {
    setChunkCount(0);
  }
}

function smoothMomentData(frames) {
  const pitchValues = frames
    .map((frame) => getField(frame, "pitch_hz", "pitchHz"))
    .filter(isFiniteNumber);
  const formantValues = [0, 1, 2, 3].map((index) =>
    frames
      .map((frame) =>
        readArrayValue(getField(frame, "formants_hz", "formantsHz"), index),
      )
      .filter(isFiniteNumber),
  );

  return {
    frameCount: frames.length,
    pitchHz: mean(pitchValues),
    pitchClarity: mean(frames.map((frame) => getField(frame, "pitch_clarity", "pitchClarity"))),
    rms: mean(frames.map((frame) => getField(frame, "rms"))),
    energy: mean(frames.map((frame) => getField(frame, "energy"))),
    hnrDb: mean(frames.map((frame) => getField(frame, "hnr_db", "hnrDb"))),
    spectralCentroidHz: mean(
      frames.map((frame) =>
        getField(frame, "spectral_centroid_hz", "spectralCentroidHz"),
      ),
    ),
    spectralBandwidthHz: mean(
      frames.map((frame) =>
        getField(frame, "spectral_bandwidth_hz", "spectralBandwidthHz"),
      ),
    ),
    spectralRolloffHz: mean(
      frames.map((frame) =>
        getField(frame, "spectral_rolloff_hz", "spectralRolloffHz"),
      ),
    ),
    spectralFlatness: mean(
      frames.map((frame) =>
        getField(frame, "spectral_flatness", "spectralFlatness"),
      ),
    ),
    spectralTiltDbPerOctave: mean(
      frames.map((frame) =>
        getField(
          frame,
          "spectral_tilt_db_per_octave",
          "spectralTiltDbPerOctave",
        ),
      ),
    ),
    zcr: mean(frames.map((frame) => getField(frame, "zcr"))),
    f1Hz: mean(formantValues[0]),
    f2Hz: mean(formantValues[1]),
    f3Hz: mean(formantValues[2]),
    f4Hz: mean(formantValues[3]),
  };
}

function summarizeChunkFields(chunk) {
  const pitch = getField(chunk, "pitch_hz", "pitchHz");
  const energy = getField(chunk, "energy");
  const spectral = getField(chunk, "spectral");
  const formants = getField(chunk, "formants");

  return {
    frameCount: getField(chunk, "frame_count", "frameCount"),
    pitchMeanHz: getField(pitch, "mean"),
    pitchMedianHz: getField(pitch, "median"),
    pitchStdHz: getField(pitch, "std"),
    energyMean: getField(energy, "mean"),
    rmsMean: getField(getField(spectral, "rms"), "mean"),
    hnrMeanDb: getField(getField(spectral, "hnr_db", "hnrDb"), "mean"),
    centroidMeanHz: getField(
      getField(spectral, "centroid_hz", "centroidHz"),
      "mean",
    ),
    bandwidthMeanHz: getField(
      getField(spectral, "bandwidth_hz", "bandwidthHz"),
      "mean",
    ),
    rolloffMeanHz: getField(
      getField(spectral, "rolloff_hz", "rolloffHz"),
      "mean",
    ),
    flatnessMean: getField(getField(spectral, "flatness"), "mean"),
    tiltMeanDbPerOctave: getField(
      getField(spectral, "tilt_db_per_octave", "tiltDbPerOctave"),
      "mean",
    ),
    zcrMean: getField(getField(spectral, "zcr"), "mean"),
    f1MeanHz: getField(
      getField(getField(formants, "f1"), "frequency_hz", "frequencyHz"),
      "mean",
    ),
    f2MeanHz: getField(
      getField(getField(formants, "f2"), "frequency_hz", "frequencyHz"),
      "mean",
    ),
    f3MeanHz: getField(
      getField(getField(formants, "f3"), "frequency_hz", "frequencyHz"),
      "mean",
    ),
    f4MeanHz: getField(
      getField(getField(formants, "f4"), "frequency_hz", "frequencyHz"),
      "mean",
    ),
  };
}

function renderMetricGrid(container, definitions) {
  container.replaceChildren(
    ...definitions.map(([key, label]) => {
      const item = document.createElement("div");
      item.className = "metric-card";
      item.dataset.metricKey = key;

      const title = document.createElement("span");
      title.className = "metric-label";
      title.textContent = label;

      const value = document.createElement("strong");
      value.className = "metric-value";
      value.textContent = "-";

      item.append(title, value);
      return item;
    }),
  );
}

function resetMetricGrid(container) {
  for (const valueElement of container.querySelectorAll(".metric-value")) {
    valueElement.textContent = "-";
  }
}

function updateMetricGrid(container, values) {
  for (const metricCard of container.querySelectorAll("[data-metric-key]")) {
    const key = metricCard.dataset.metricKey;
    const valueElement = metricCard.querySelector(".metric-value");
    valueElement.textContent = formatMetricValue(values[key]);
  }
}

function setStatus(text) {
  elements.statusText.textContent = text;
}

function setChunkCount(value) {
  elements.chunkCountText.textContent = String(value);
}

function mean(values) {
  const numericValues = values.filter(isFiniteNumber);
  if (numericValues.length === 0) {
    return null;
  }

  let total = 0;
  for (const value of numericValues) {
    total += value;
  }
  return total / numericValues.length;
}

function formatMetricValue(value) {
  if (!isFiniteNumber(value)) {
    if (value === 0) {
      return "0";
    }
    return "-";
  }

  if (Number.isInteger(value)) {
    return String(value);
  }

  return value.toFixed(2);
}

function isFiniteNumber(value) {
  return typeof value === "number" && Number.isFinite(value);
}

function readArrayValue(value, index) {
  if (!Array.isArray(value)) {
    return null;
  }

  return value[index] ?? null;
}

function getField(value, ...keys) {
  if (value == null) {
    return null;
  }

  for (const key of keys) {
    if (key in value) {
      return value[key];
    }
  }

  return null;
}
