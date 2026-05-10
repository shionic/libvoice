import type { PitchFrame, MidiData } from './types';
import { PITCH_MIN, PITCH_MAX, PITCH_RANGE, midiToNoteName, freqToMidi } from './types';

export const PX_PER_MS = 0.22;
export const LANE_H = 14;
export const LABEL_W = 58;
export const CURSOR_X = 320;
const SCROLL_EASE = 0.12;

const C = {
  bg: '#0a0a0f',
  bgLane: '#0c0c16',
  bgLaneMark: '#0f0f1e',
  grid: 'rgba(255,255,255,0.05)',
  barLine: 'rgba(255,255,255,0.18)',
  beatLine: 'rgba(255,255,255,0.07)',
  noteBlock: '#7c6bff',
  noteHit: '#00ff88',
  warn: '#ffb700',
  error: '#ff4060',
  pitchLine: '#00e5ff',
  cursor: 'rgba(0,229,255,0.55)',
  cursorLine: 'rgba(0,229,255,0.20)',
  labelBg: '#0a0a0f',
  labelText: '#555570',
  labelTextC: '#8888aa',
};

export interface RenderState {
  pitchFrames: PitchFrame[];
  midiData: MidiData | null;
  isPlaying: boolean;
  isRecording: boolean;
  playbackMs: number;
  captureMs: number;
  scrollOffsetMs: number;
  scrollMode: 'follow' | 'free';
  bpm: number;
  timeSigNum: number;
  timeSigDen: number;
  viewportW: number;
  viewportH: number;
  currentPitch: PitchFrame | null;
}

export class PitchCanvas {
  private canvas: HTMLCanvasElement;
  private ctx: CanvasRenderingContext2D;
  private labelCanvas: HTMLCanvasElement;
  private labelCtx: CanvasRenderingContext2D;
  private state: RenderState;
  private animFrame = 0;
  private targetScrollMs = 0;
  private currentScrollMs = 0;
  private dpr: number = window.devicePixelRatio || 1;
  private hitCache: Map<number, number> = new Map();

  constructor(canvas: HTMLCanvasElement, labelCanvas: HTMLCanvasElement, initialState: RenderState) {
    this.canvas = canvas;
    this.labelCanvas = labelCanvas;
    this.ctx = canvas.getContext('2d', { alpha: false })!;
    this.labelCtx = labelCanvas.getContext('2d', { alpha: false })!;
    this.state = initialState;
  }

  update(partial: Partial<RenderState>) { Object.assign(this.state, partial); }
  addPitchFrame(frame: PitchFrame) { this.state.pitchFrames.push(frame); }
  updateHitCache(noteIdx: number, hitPct: number) { this.hitCache.set(noteIdx, hitPct); }
  start() { this.loop(); }
  stop() { cancelAnimationFrame(this.animFrame); }

  resize(w: number) {
    this.state.viewportW = w;
    const totalH = PITCH_RANGE * LANE_H;

    this.canvas.width = (w - LABEL_W) * this.dpr;
    this.canvas.height = totalH * this.dpr;
    this.canvas.style.width = `${w - LABEL_W}px`;
    this.canvas.style.height = `${totalH}px`;
    this.ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);

    this.labelCanvas.width = LABEL_W * this.dpr;
    this.labelCanvas.height = totalH * this.dpr;
    this.labelCanvas.style.width = `${LABEL_W}px`;
    this.labelCanvas.style.height = `${totalH}px`;
    this.labelCtx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);

    this.renderLabels();
  }

  private msToX(ms: number) { return (ms - this.currentScrollMs) * PX_PER_MS; }
  private midiToY(midi: number) { return (PITCH_MAX - midi) * LANE_H; }

  private loop() {
    this.animFrame = requestAnimationFrame(() => this.loop());
    const s = this.state;
    this.targetScrollMs = s.scrollMode === 'follow'
      ? (s.isPlaying ? s.playbackMs : s.captureMs) - CURSOR_X / PX_PER_MS
      : s.scrollOffsetMs;
    this.currentScrollMs += (this.targetScrollMs - this.currentScrollMs) * SCROLL_EASE;
    this.render();
  }

  private render() {
    const s = this.state;
    const W = s.viewportW - LABEL_W;
    const H = PITCH_RANGE * LANE_H;
    const ctx = this.ctx;

    ctx.fillStyle = C.bg;
    ctx.fillRect(0, 0, W, H);

    for (let midi = PITCH_MIN; midi <= PITCH_MAX; midi++) {
      const y = this.midiToY(midi);
      const noteClass = midi % 12;
      const isC = noteClass === 0;
      const isBlack = [1, 3, 6, 8, 10].includes(noteClass);
      ctx.fillStyle = isC ? C.bgLaneMark : isBlack ? C.bg : C.bgLane;
      ctx.fillRect(0, y, W, LANE_H);
      ctx.fillStyle = C.grid;
      ctx.fillRect(0, y + LANE_H - 1, W, 1);
    }

    const beatMs = 60000 / s.bpm;
    const barMs = beatMs * s.timeSigNum;
    const startMs = this.currentScrollMs - barMs;
    const endMs = this.currentScrollMs + W / PX_PER_MS + barMs;
    for (let b = Math.floor(startMs / beatMs); b <= Math.ceil(endMs / beatMs); b++) {
      const x = this.msToX(b * beatMs);
      if (x < -10 || x > W + 10) continue;
      const isBar = b % s.timeSigNum === 0;
      ctx.strokeStyle = isBar ? C.barLine : C.beatLine;
      ctx.lineWidth = isBar ? 1.5 : 1;
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, H);
      ctx.stroke();
    }

    if (s.midiData) {
      const visStart = this.currentScrollMs;
      const visEnd = visStart + W / PX_PER_MS;
      for (let i = 0; i < s.midiData.notes.length; i++) {
        const note = s.midiData.notes[i];
        if (note.end_ms < visStart || note.start_ms > visEnd) continue;
        if (note.pitch < PITCH_MIN || note.pitch > PITCH_MAX) continue;
        const x = this.msToX(note.start_ms);
        const w = Math.max(2, (note.end_ms - note.start_ms) * PX_PER_MS - 1);
        const y = this.midiToY(note.pitch) + 1;
        const h = LANE_H - 2;
        const hitPct = this.hitCache.get(i) ?? -1;
        const isPast = note.end_ms < (s.isPlaying ? s.playbackMs : s.captureMs);
        let base = C.noteBlock;
        let alpha = isPast ? 0.35 : 0.75;
        if (hitPct >= 0 && isPast) {
          base = hitPct >= 75 ? C.noteHit : hitPct >= 40 ? C.warn : C.error;
          alpha = 0.85;
        }
        ctx.globalAlpha = alpha;
        ctx.fillStyle = base;
        ctx.fillRect(x, y, w, h);
      }
      ctx.globalAlpha = 1;
    }

    const frames = s.pitchFrames;
    if (frames.length > 1) {
      const visStart = this.currentScrollMs;
      const visEnd = visStart + W / PX_PER_MS;
      let inSegment = false;
      ctx.save();
      ctx.lineJoin = 'round';
      ctx.lineCap = 'round';
      for (let i = 0; i < frames.length; i++) {
        const f = frames[i];
        if (f.timestamp_ms < visStart - 500 || f.timestamp_ms > visEnd + 500) continue;
        if (f.frequency <= 0 || f.confidence < 0.08) {
          if (inSegment) { ctx.stroke(); inSegment = false; }
          continue;
        }
        const midiF = freqToMidi(f.frequency);
        const x = this.msToX(f.timestamp_ms);
        const y = this.midiToY(Math.round(midiF)) + LANE_H / 2 - (midiF - Math.round(midiF)) * LANE_H;
        if (!inSegment) {
          ctx.beginPath();
          ctx.strokeStyle = C.pitchLine;
          ctx.globalAlpha = Math.min(1, f.confidence * 1.4);
          ctx.lineWidth = 2.5;
          ctx.moveTo(x, y);
          inSegment = true;
        } else {
          ctx.lineTo(x, y);
        }
      }
      if (inSegment) ctx.stroke();
      ctx.restore();
    }

    if (s.scrollMode === 'follow') {
      const x = CURSOR_X;
      const grad = ctx.createLinearGradient(x - 40, 0, x + 2, 0);
      grad.addColorStop(0, 'transparent');
      grad.addColorStop(1, C.cursorLine);
      ctx.fillStyle = grad;
      ctx.fillRect(x - 40, 0, 42, H);
      ctx.strokeStyle = C.cursor;
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, H);
      ctx.stroke();
    }
  }

  renderLabels() {
    const ctx = this.labelCtx;
    const W = LABEL_W;
    const H = PITCH_RANGE * LANE_H;
    ctx.fillStyle = C.labelBg;
    ctx.fillRect(0, 0, W, H);
    for (let midi = PITCH_MIN; midi <= PITCH_MAX; midi++) {
      const y = this.midiToY(midi);
      const noteClass = midi % 12;
      const isC = noteClass === 0;
      const isBlack = [1, 3, 6, 8, 10].includes(noteClass);
      if (isC || (!isBlack && midi % 3 === 0)) {
        ctx.font = isC ? `700 11px 'Space Mono', monospace` : `400 10px 'Space Mono', monospace`;
        ctx.fillStyle = isC ? C.labelTextC : C.labelText;
        ctx.textAlign = 'right';
        ctx.textBaseline = 'middle';
        ctx.fillText(midiToNoteName(midi), W - 8, y + LANE_H / 2);
      }
      ctx.fillStyle = C.grid;
      ctx.fillRect(W - 3, y + LANE_H - 1, 3, 1);
    }
    ctx.fillStyle = 'rgba(255,255,255,0.08)';
    ctx.fillRect(W - 1, 0, 1, H);
  }

  getScrollMs() { return this.currentScrollMs; }
  setScrollMode(mode: 'follow' | 'free') { this.state.scrollMode = mode; }
  setScrollOffset(ms: number) { this.state.scrollOffsetMs = ms; this.targetScrollMs = ms; }
  nudgeScroll(deltaMs: number) {
    if (this.state.scrollMode === 'free') {
      this.state.scrollOffsetMs += deltaMs;
      this.targetScrollMs = this.state.scrollOffsetMs;
    }
  }
}
