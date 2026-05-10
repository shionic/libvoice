<script setup lang="ts">
import { computed } from 'vue';

const props = defineProps<{
  isRecording: boolean;
  isPlaying: boolean;
  bpm: number;
  timeSigNum: number;
  timeSigDen: number;
  hasMidi: boolean;
  hasAudio: boolean;
  hasRecording: boolean;
  scrollMode: 'follow' | 'free';
  currentBeat: number;
  playbackMs: number;
  captureMs: number;
}>();

const emit = defineEmits<{
  loadMidi: [];
  loadAudio: [];
  toggleRecord: [];
  play: [];
  stop: [];
  review: [];
  bpmChange: [number];
  timeSigChange: [number, number];
  scrollModeChange: ['follow' | 'free'];
}>();

const displayMs = computed(() => (props.isPlaying ? props.playbackMs : props.captureMs));
const timecode = computed(() => {
  const ms = displayMs.value;
  const s = Math.floor(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}.${String(Math.floor(ms % 1000)).padStart(3, '0')}`;
});
const beatText = computed(() => `BAR ${Math.floor(props.currentBeat / props.timeSigNum) + 1} . ${(props.currentBeat % props.timeSigNum) + 1}`);
const statusLabel = computed(() => (props.isRecording ? 'Recording' : props.isPlaying ? 'Playing' : 'Ready'));
const statusClass = computed(() => (props.isRecording ? 'rec' : props.isPlaying ? 'play' : 'ready'));

function onBpmChange(event: Event) {
  emit('bpmChange', Number((event.target as HTMLInputElement).value));
}

function onTimeSigChange(event: Event) {
  const [n, d] = (event.target as HTMLSelectElement).value.split('/').map(Number);
  emit('timeSigChange', n, d);
}
</script>

<template>
  <div class="transport">
    <div class="t-group">
      <button class="btn-secondary t-load" :class="{ loaded: hasMidi }" @click="emit('loadMidi')">MIDI</button>
      <button class="btn-secondary t-load" :class="{ loaded: hasAudio }" @click="emit('loadAudio')">Audio</button>
    </div>
    <div class="sep"></div>
    <div class="t-group transport-meta">
      <div class="t-field">
        <label for="bpm-input">BPM</label>
        <input id="bpm-input" class="t-bpm" type="number" min="20" max="300" :value="bpm" @change="onBpmChange" />
      </div>
      <div class="t-field">
        <label for="time-sig-input">TIME SIG</label>
        <select id="time-sig-input" class="t-timesig" :value="`${timeSigNum}/${timeSigDen}`" @change="onTimeSigChange">
          <option value="4/4">4/4</option>
          <option value="3/4">3/4</option>
          <option value="2/4">2/4</option>
          <option value="6/8">6/8</option>
          <option value="5/4">5/4</option>
          <option value="7/8">7/8</option>
        </select>
      </div>
    </div>
    <div class="sep"></div>
    <div class="t-group">
      <button class="btn-danger t-rec" :class="{ active: isRecording }" @click="emit('toggleRecord')">REC</button>
      <button class="btn-secondary t-play" :disabled="isRecording || (!hasMidi && !hasAudio)" @click="emit('play')">▶</button>
      <button class="btn-secondary t-stop" :disabled="!isPlaying && !isRecording" @click="emit('stop')">■</button>
    </div>
    <div class="sep"></div>
    <div class="t-timecode">
      <div class="t-time mono">{{ timecode }}</div>
      <div class="t-beat mono">{{ beatText }}</div>
    </div>
    <div class="sep"></div>
    <div class="t-status">
      <div class="t-status-dot" :class="statusClass"></div>
      <span class="t-status-label">{{ statusLabel }}</span>
    </div>
    <div class="t-spacer"></div>
    <div class="t-group">
      <button class="btn-ghost t-scroll-btn" @click="emit('scrollModeChange', scrollMode === 'follow' ? 'free' : 'follow')">{{ scrollMode === 'follow' ? 'Follow' : 'Free' }}</button>
      <button class="btn-ghost t-stats-btn" :disabled="!hasRecording" @click="emit('review')">Stats</button>
    </div>
  </div>
</template>
