export interface PitchFrame {
  timestamp_ms: number;
  frequency: number;
  confidence: number;
  note: string | null;
  cents_deviation: number;
  midi_note: number | null;
}

export interface MidiNote {
  start_tick: number;
  end_tick: number;
  start_ms: number;
  end_ms: number;
  pitch: number;
  velocity: number;
  channel: number;
}

export interface MidiData {
  notes: MidiNote[];
  ticks_per_beat: number;
  tempo_bpm: number;
  time_sig_num: number;
  time_sig_den: number;
  duration_ms: number;
  total_ticks: number;
}

export interface NoteAccuracy {
  midi_note: number;
  note_name: string;
  hit_percentage: number;
  avg_deviation_cents: number;
  max_deviation_cents: number;
}

export interface RecordingData {
  pitch_frames: PitchFrame[];
  duration_ms: number;
  note_accuracies: NoteAccuracy[];
  overall_accuracy: number;
}

export type AppMode = 'idle' | 'recording' | 'playback' | 'review';

export interface AppState {
  mode: AppMode;
  isCapturing: boolean;
  isPlaying: boolean;
  bpm: number;
  timeSigNum: number;
  timeSigDen: number;
  midiData: MidiData | null;
  audioFilePath: string | null;
  midiFilePath: string | null;
  pitchHistory: PitchFrame[];
  currentPitch: PitchFrame | null;
  playbackMs: number;
  currentBeat: number;
  recordingData: RecordingData | null;
  captureMs?: number;
  scrollMode: 'follow' | 'free';
  scrollOffsetMs: number;
}

export const NOTE_NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];

export function midiToNoteName(midi: number): string {
  const octave = Math.floor(midi / 12) - 1;
  const note = NOTE_NAMES[midi % 12];
  return `${note}${octave}`;
}

export function freqToMidi(freq: number): number {
  return 69 + 12 * Math.log2(freq / 440);
}

export const PITCH_MIN = 36;
export const PITCH_MAX = 84;
export const PITCH_RANGE = PITCH_MAX - PITCH_MIN;
