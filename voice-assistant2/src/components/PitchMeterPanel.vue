<script setup lang="ts">
import type { PitchFrame } from '../types';
import { computed } from 'vue';
import { midiToNoteName } from '../types';

const props = defineProps<{ frame: PitchFrame | null }>();

const visible = computed(() => !!props.frame && props.frame.frequency > 0 && props.frame.confidence > 0.08);
const note = computed(() => (visible.value && props.frame?.midi_note != null ? midiToNoteName(props.frame.midi_note) : '—'));
const freq = computed(() => (visible.value && props.frame ? `${props.frame.frequency.toFixed(1)} Hz` : '—'));
const cents = computed(() => (visible.value && props.frame ? `${props.frame.cents_deviation >= 0 ? '+' : ''}${props.frame.cents_deviation.toFixed(0)}` : '±0'));
const confWidth = computed(() => `${Math.min(100, (props.frame?.confidence ?? 0) * 100)}%`);
const centsValue = computed(() => (visible.value && props.frame ? props.frame.cents_deviation : 0));
const centsClass = computed(() => {
  const abs = Math.abs(centsValue.value);
  if (abs < 8) return 'in-tune';
  return centsValue.value > 0 ? 'sharp' : 'flat';
});
const needleStyle = computed(() => {
  const clamped = Math.max(-50, Math.min(50, centsValue.value));
  const angle = (clamped / 50) * 45;
  return { transform: `rotate(${angle}deg)` };
});
</script>

<template>
  <div class="pitch-meter">
    <div class="meter-top">
      <div class="meter-note-wrap">
        <div class="meter-note" :class="{ silent: !visible }">{{ note }}</div>
        <div class="meter-freq mono">{{ freq }}</div>
      </div>
      <div class="meter-arc-wrap" aria-hidden="true">
        <div class="meter-arc"></div>
        <div class="meter-needle" :style="needleStyle"></div>
        <div class="meter-pivot"></div>
      </div>
    </div>
    <div class="meter-cents-row">
      <span class="meter-cents-label mono">CENTS</span>
      <span class="meter-cents mono" :class="centsClass">{{ cents }}</span>
    </div>
    <div class="meter-conf-bar"><div class="meter-conf-fill" :style="{ width: confWidth }"></div></div>
  </div>
</template>
