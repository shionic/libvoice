<script setup lang="ts">
import { computed, onMounted, onBeforeUnmount, reactive, ref } from 'vue';
import type { AppState, PitchFrame, MidiData } from './types';
import { midiToNoteName } from './types';
import { commands, listenEvent, openFileDialog } from './tauri';
import { PitchCanvas, LANE_H, PX_PER_MS } from './PitchCanvas';
import TransportBar from './components/TransportBar.vue';
import PitchMeterPanel from './components/PitchMeterPanel.vue';
import StatsPanel from './components/StatsPanel.vue';

const rootEl = ref<HTMLElement | null>(null);
const canvasWrapper = ref<HTMLElement | null>(null);
const mainCanvas = ref<HTMLCanvasElement | null>(null);
const labelCanvas = ref<HTMLCanvasElement | null>(null);

const state = reactive<AppState>({
  mode: 'idle', isCapturing: false, isPlaying: false, bpm: 120, timeSigNum: 4, timeSigDen: 4,
  midiData: null, audioFilePath: null, midiFilePath: null, pitchHistory: [], currentPitch: null,
  playbackMs: 0, currentBeat: 0, recordingData: null, scrollMode: 'follow', scrollOffsetMs: 0, captureMs: 0,
});

let pitchCanvas: PitchCanvas | null = null;
let mockStopFn: (() => void) | undefined;
let playbackStartWall = 0;
let captureStartWall = 0;
let playbackRAF = 0;
const unlisten: Array<() => void> = [];
const showStats = ref(false);

const midiFileName = computed(() => state.midiFilePath?.split(/[\\/]/).pop() ?? '—');
const audioFileName = computed(() => state.audioFilePath?.split(/[\\/]/).pop() ?? '—');
const noteCountText = computed(() => `${state.midiData?.notes.length ?? 0} notes`);
const statusHint = computed(() => {
  if (state.isCapturing) return 'Recording — sing along with the highlighted notes';
  if (state.recordingData) return 'Recording stopped — press Stats to review';
  return 'Load a MIDI file and press REC to begin';
});

function makeMockMidi(): MidiData {
  const bpm = 100;
  const beatMs = 60000 / bpm;
  const scale = [60, 62, 64, 65, 67, 69, 71, 72, 71, 69, 67, 65, 64, 62, 60];
  return {
    notes: scale.map((pitch, i) => ({ start_tick: Math.round(i * 480 * 1.5), end_tick: Math.round((i + 1) * 480 * 1.5) - 48, start_ms: i * beatMs * 1.5, end_ms: i * beatMs * 1.5 + beatMs * 1.2, pitch, velocity: 90, channel: 0 })),
    ticks_per_beat: 480, tempo_bpm: bpm, time_sig_num: 4, time_sig_den: 4, duration_ms: scale.length * beatMs * 1.5 + beatMs, total_ticks: Math.round(scale.length * 480 * 1.5 + 480),
  };
}

function startMockPitch(onFrame: (f: PitchFrame) => void): () => void {
  let t = 0; let baseMidi = 60; let trend = 0.05;
  const interval = setInterval(() => {
    t += 0.05; baseMidi += trend; if (baseMidi > 72 || baseMidi < 52) trend *= -1;
    const midi = baseMidi + Math.sin(t * 6) * 0.4 + (Math.random() - 0.5) * 0.3;
    onFrame({ timestamp_ms: performance.now(), frequency: 440 * Math.pow(2, (midi - 69) / 12), confidence: 0.8, note: midiToNoteName(Math.round(midi)), cents_deviation: (midi - Math.round(midi)) * 100, midi_note: Math.round(midi) });
  }, 23);
  return () => clearInterval(interval);
}

// Track hit counts per note index for real-time accuracy
const noteHitStats = new Map<number, { total: number; hits: number }>();

function onPitchFrame(frame: PitchFrame) {
  if (!state.isCapturing) return;
  
  // Synchronize timestamp with playback if playing
  let syncedTimestamp = frame.timestamp_ms;
  if (state.isPlaying && playbackStartWall > 0) {
    // Use playback time instead of capture time
    syncedTimestamp = performance.now() - playbackStartWall;
  } else if (captureStartWall > 0) {
    // Use capture time relative to when capture started
    syncedTimestamp = performance.now() - captureStartWall;
  }
  
  // Create synced frame
  const syncedFrame = { ...frame, timestamp_ms: syncedTimestamp };
  
  state.pitchHistory.push(syncedFrame);
  state.currentPitch = syncedFrame;
  state.captureMs = syncedTimestamp;
  pitchCanvas?.addPitchFrame(syncedFrame);
  
  // Real-time hit detection: check which MIDI notes are active at this timestamp
  if (state.midiData && syncedFrame.confidence > 0.1) {
    const currentMs = syncedTimestamp;
    
    state.midiData.notes.forEach((note, idx) => {
      // Check if this frame falls within the note's time range
      if (currentMs >= note.start_ms && currentMs <= note.end_ms) {
        // Initialize stats for this note if not exists
        if (!noteHitStats.has(idx)) {
          noteHitStats.set(idx, { total: 0, hits: 0 });
        }
        
        const stats = noteHitStats.get(idx)!;
        stats.total++;
        
        // Check if the sung pitch matches the MIDI note
        if (syncedFrame.midi_note === note.pitch) {
          stats.hits++;
        }
        
        // Calculate and update hit percentage
        const hitPct = (stats.hits / stats.total) * 100;
        pitchCanvas?.updateHitCache(idx, hitPct);
      }
    });
  }
  
  pitchCanvas?.update({ captureMs: syncedTimestamp, currentPitch: syncedFrame });
}

async function bindTauriEvents() {
  unlisten.push(await listenEvent<PitchFrame>('pitch-frame', onPitchFrame));
  unlisten.push(await listenEvent('playback-started', () => { 
    playbackStartWall = performance.now(); 
    if (!captureStartWall) captureStartWall = playbackStartWall;
    tickPlayback(); 
  }));
  unlisten.push(await listenEvent('playback-stopped', () => { cancelAnimationFrame(playbackRAF); state.isPlaying = false; }));
}

async function toggleRecord() { state.isCapturing ? stopRecording() : startRecording(); }
async function startRecording() {
  if (state.isCapturing) return;
  state.isCapturing = true;
  state.pitchHistory = [];
  noteHitStats.clear(); // Clear hit stats for new recording
  captureStartWall = performance.now(); // Track when capture started
  pitchCanvas?.update({ pitchFrames: [], isRecording: true, captureMs: 0, scrollMode: 'follow' });
  pitchCanvas?.setScrollMode('follow');
  try { await commands.startAudioCapture(); } catch {}
  // Start playback if audio file is loaded
  if (state.audioFilePath && !state.isPlaying) {
    state.isPlaying = true;
    state.playbackMs = 0;
    playbackStartWall = captureStartWall; // Sync playback with capture
    pitchCanvas?.update({ isPlaying: true, playbackMs: 0 });
    try { await commands.startPlayback(); } catch { tickPlayback(); }
  }
}
async function stopRecording() {
  if (!state.isCapturing) return;
  state.isCapturing = false;
  try { await commands.stopAudioCapture(); } catch {}
  try {
    state.recordingData = await commands.getRecordingData();
    // Populate hit cache with accuracy data for visual feedback
    // Backend now returns per-note-instance accuracy (same order as MIDI notes)
    if (state.recordingData && state.midiData) {
      state.recordingData.note_accuracies.forEach((acc, idx) => {
        pitchCanvas?.updateHitCache(idx, acc.hit_percentage);
      });
    }
  } catch {}
  pitchCanvas?.update({ isRecording: false });
}
async function play() { 
  if (state.isPlaying) return; 
  state.isPlaying = true; 
  state.playbackMs = 0; 
  playbackStartWall = performance.now();
  pitchCanvas?.update({ isPlaying: true, playbackMs: 0, scrollMode: 'follow' }); 
  pitchCanvas?.setScrollMode('follow'); 
  // Start audio capture for voice tracking during playback
  if (!state.isCapturing) {
    state.isCapturing = true;
    state.pitchHistory = [];
    noteHitStats.clear();
    captureStartWall = playbackStartWall; // Sync capture with playback
    pitchCanvas?.update({ isRecording: true, captureMs: 0 });
    try { await commands.startAudioCapture(); } catch {}
  }
  try { await commands.startPlayback(); } catch { tickPlayback(); } 
}
async function stop() { 
  if (state.isPlaying) { 
    state.isPlaying = false; 
    cancelAnimationFrame(playbackRAF); 
    playbackStartWall = 0;
    try { await commands.stopPlayback(); } catch {} 
    pitchCanvas?.update({ isPlaying: false }); 
  } 
  if (state.isCapturing) {
    captureStartWall = 0;
    await stopRecording();
  }
}
function tickPlayback() { const ms = performance.now() - playbackStartWall; state.playbackMs = ms; state.currentBeat = Math.floor(ms / (60000 / state.bpm)); pitchCanvas?.update({ playbackMs: ms }); const duration = state.midiData?.duration_ms ?? Infinity; if (ms < duration + 2000 && state.isPlaying) playbackRAF = requestAnimationFrame(tickPlayback); }

async function loadMidi() { const path = await openFileDialog([{ name: 'MIDI', extensions: ['mid', 'midi'] }]); if (!path) return; const midi = await commands.loadMidiFile(path); state.midiData = midi; state.bpm = midi.tempo_bpm; state.timeSigNum = midi.time_sig_num; state.timeSigDen = midi.time_sig_den; pitchCanvas?.update({ midiData: midi, bpm: midi.tempo_bpm, timeSigNum: midi.time_sig_num, timeSigDen: midi.time_sig_den }); state.midiFilePath = path; }
async function loadAudio() { const path = await openFileDialog([{ name: 'Audio', extensions: ['mp3', 'wav', 'ogg', 'flac', 'm4a'] }]); if (!path) return; await commands.loadAudioFile(path); state.audioFilePath = path; }
async function setBpm(bpm: number) { state.bpm = bpm; pitchCanvas?.update({ bpm }); try { await commands.setBpm(bpm); } catch {} }
async function setTimeSig(num: number, den: number) { state.timeSigNum = num; state.timeSigDen = den; pitchCanvas?.update({ timeSigNum: num, timeSigDen: den }); try { await commands.setTimeSignature(num, den); } catch {} }
function setScrollMode(mode: 'follow' | 'free') { state.scrollMode = mode; pitchCanvas?.setScrollMode(mode); }
function review() { showStats.value = !showStats.value; }

function setupScroll() {
  if (!canvasWrapper.value) return;
  const el = canvasWrapper.value;
  let dragging = false; let dragStartX = 0; let dragStartScrollMs = 0;
  el.addEventListener('mousedown', (e) => { if (state.scrollMode !== 'free') return; dragging = true; dragStartX = e.clientX; dragStartScrollMs = pitchCanvas?.getScrollMs() ?? 0; });
  window.addEventListener('mousemove', (e) => { if (!dragging) return; const ms = Math.max(0, dragStartScrollMs - (e.clientX - dragStartX) / PX_PER_MS); pitchCanvas?.setScrollOffset(ms); state.scrollOffsetMs = ms; });
  window.addEventListener('mouseup', () => (dragging = false));
  el.addEventListener('wheel', (e) => { e.preventDefault(); if (state.scrollMode === 'follow') setScrollMode('free'); pitchCanvas?.nudgeScroll((e.deltaX + e.deltaY) / PX_PER_MS); }, { passive: false });
}

onMounted(() => {
  if (!canvasWrapper.value || !mainCanvas.value || !labelCanvas.value) return;
  pitchCanvas = new PitchCanvas(mainCanvas.value, labelCanvas.value, {
    pitchFrames: state.pitchHistory, midiData: state.midiData, isPlaying: false, isRecording: false,
    playbackMs: 0, captureMs: 0, scrollOffsetMs: 0, scrollMode: 'follow', bpm: state.bpm, timeSigNum: state.timeSigNum, timeSigDen: state.timeSigDen,
    viewportW: canvasWrapper.value.clientWidth, viewportH: LANE_H * 48, currentPitch: null,
  });
  pitchCanvas.start();
  pitchCanvas.resize(canvasWrapper.value.clientWidth);
  new ResizeObserver(() => pitchCanvas?.resize(canvasWrapper.value?.clientWidth ?? 800)).observe(canvasWrapper.value);
  setupScroll();

  if ('__TAURI__' in window) bindTauriEvents();
  else {
    const midi = makeMockMidi(); state.midiData = midi; state.bpm = midi.tempo_bpm; pitchCanvas.update({ midiData: midi, bpm: midi.tempo_bpm });
    mockStopFn = startMockPitch(onPitchFrame);
  }
});

onBeforeUnmount(() => { pitchCanvas?.stop(); mockStopFn?.(); unlisten.forEach((fn) => fn()); cancelAnimationFrame(playbackRAF); });
</script>

<template>
  <div ref="rootEl" class="app-root" role="application" aria-label="Vocal Assistant">
    <div class="app-topbar" role="banner">
      <div class="app-brand" aria-hidden="true">
        <span class="app-brand-icon">◎</span>
        <span class="app-brand-name">VOCAL</span>
      </div>
      <div class="app-transport">
        <TransportBar
          :is-recording="state.isCapturing"
          :is-playing="state.isPlaying"
          :bpm="state.bpm"
          :time-sig-num="state.timeSigNum"
          :time-sig-den="state.timeSigDen"
          :has-midi="!!state.midiData"
          :has-audio="!!state.audioFilePath"
          :has-recording="!!state.recordingData"
          :scroll-mode="state.scrollMode"
          :current-beat="state.currentBeat"
          :playback-ms="state.playbackMs"
          :capture-ms="state.captureMs ?? 0"
          @load-midi="loadMidi"
          @load-audio="loadAudio"
          @toggle-record="toggleRecord"
          @play="play"
          @stop="stop"
          @review="review"
          @bpm-change="setBpm"
          @time-sig-change="setTimeSig"
          @scroll-mode-change="setScrollMode"
        />
      </div>
    </div>

    <div class="app-content">
      <aside class="app-sidebar" aria-label="Pitch monitor">
        <PitchMeterPanel :frame="state.currentPitch" />
        <div class="sidebar-info">
          <div class="info-item">
            <span class="info-label">MIDI FILE</span>
            <span class="info-value mono" id="midi-filename">{{ midiFileName }}</span>
          </div>
          <div class="info-item">
            <span class="info-label">AUDIO FILE</span>
            <span class="info-value mono" id="audio-filename">{{ audioFileName }}</span>
          </div>
          <div class="info-item">
            <span class="info-label">NOTES</span>
            <span class="info-value mono">{{ noteCountText }}</span>
          </div>
        </div>
        <div class="sidebar-legend" aria-label="Legend">
          <div class="legend-row"><span class="legend-sw legend-midi"></span>MIDI note</div>
          <div class="legend-row"><span class="legend-sw legend-hit"></span>Hit note</div>
          <div class="legend-row"><span class="legend-sw legend-partial"></span>Partial hit</div>
          <div class="legend-row"><span class="legend-sw legend-missed"></span>Missed</div>
          <div class="legend-row"><span class="legend-sw legend-pitch"></span>Your pitch</div>
        </div>
      </aside>

      <main class="app-main" aria-label="Musical notation view">
        <div ref="canvasWrapper" class="canvas-scroll-wrapper" id="canvas-scroll-wrapper" tabindex="0">
          <div class="canvas-inner" id="canvas-inner">
            <canvas ref="labelCanvas" id="label-canvas" class="label-canvas" aria-hidden="true"></canvas>
            <canvas ref="mainCanvas" id="main-canvas" class="main-canvas" aria-hidden="true"></canvas>
          </div>
        </div>
      </main>

      <div class="app-stats" :class="{ hidden: !showStats }" id="stats-wrapper">
        <StatsPanel :data="state.recordingData" />
      </div>
    </div>

    <footer class="app-statusbar">
      <span class="status-hint">{{ statusHint }}</span>
      <div class="status-right">
        <span class="status-latency mono">FFT: 4096pt · 44.1kHz</span>
        <span class="status-latency mono">HPS · 5 harmonics</span>
      </div>
    </footer>
  </div>
</template>