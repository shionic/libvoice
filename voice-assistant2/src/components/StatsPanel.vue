<script setup lang="ts">
import type { RecordingData } from '../types';

defineProps<{ data: RecordingData | null }>();

const accuracyColor = (pct: number) => (pct >= 75 ? 'var(--hit)' : pct >= 50 ? 'var(--warn)' : 'var(--error)');
</script>

<template>
  <div v-if="data" class="stats-panel">
    <div class="stats-header">
      <h2 class="stats-title">Session Results</h2>
      <div class="stats-overall" :style="{ color: accuracyColor(data.overall_accuracy) }">
        <span class="stats-overall-num">{{ data.overall_accuracy.toFixed(0) }}<span class="stats-pct">%</span></span>
        <span class="stats-overall-label">Overall Accuracy</span>
      </div>
    </div>
    <div class="stats-summary">
      <div class="s-stat">
        <span class="s-stat-val">{{ Math.max(0, Math.floor(data.duration_ms / 60000)) }}:{{ String(Math.floor((data.duration_ms % 60000) / 1000)).padStart(2, '0') }}</span>
        <span class="s-stat-lbl">Duration</span>
      </div>
      <div class="s-stat">
        <span class="s-stat-val">{{ data.pitch_frames.length }}</span>
        <span class="s-stat-lbl">Pitch frames</span>
      </div>
      <div class="s-stat">
        <span class="s-stat-val">{{ data.note_accuracies.length }}</span>
        <span class="s-stat-lbl">Notes</span>
      </div>
    </div>
    <div class="stats-section-title">Per-note Accuracy</div>
    <div class="stats-notes">
      <div class="stats-note-row" v-for="n in data.note_accuracies" :key="`${n.midi_note}-${n.note_name}`">
        <span class="stats-note-name mono">{{ n.note_name }}</span>
        <div class="stats-bar-wrap"><div class="stats-bar-fill" :style="{ width: `${Math.round(n.hit_percentage)}%`, background: accuracyColor(n.hit_percentage) }"></div></div>
        <span class="stats-hit-pct mono" :style="{ color: accuracyColor(n.hit_percentage) }">{{ n.hit_percentage.toFixed(0) }}%</span>
        <span class="stats-dev mono">±{{ n.avg_deviation_cents.toFixed(0) }}¢</span>
      </div>
    </div>
  </div>
</template>
