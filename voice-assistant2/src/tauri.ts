import type { MidiData, RecordingData } from './types';

const isTauri = typeof window !== 'undefined' && '__TAURI__' in window;

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (isTauri) {
    const { invoke: tauriInvoke } = await import('@tauri-apps/api/core');
    return tauriInvoke<T>(cmd, args);
  }
  return mockInvoke<T>(cmd, args);
}

async function mockInvoke<T>(cmd: string, _args?: Record<string, unknown>): Promise<T> {
  switch (cmd) {
    case 'get_audio_devices': return ['Default Microphone'] as unknown as T;
    case 'get_recording_data':
      return { pitch_frames: [], duration_ms: 0, note_accuracies: [], overall_accuracy: 0 } as unknown as T;
    default: return undefined as unknown as T;
  }
}

export const commands = {
  getAudioDevices: () => invoke<string[]>('get_audio_devices'),
  startAudioCapture: () => invoke<void>('start_audio_capture'),
  stopAudioCapture: () => invoke<void>('stop_audio_capture'),
  loadMidiFile: (path: string) => invoke<MidiData>('load_midi_file', { path }),
  loadAudioFile: (path: string) => invoke<void>('load_audio_file', { path }),
  startPlayback: () => invoke<void>('start_playback'),
  stopPlayback: () => invoke<void>('stop_playback'),
  setBpm: (bpm: number) => invoke<void>('set_bpm', { bpm }),
  setTimeSignature: (num: number, den: number) => invoke<void>('set_time_signature', { num, den }),
  getRecordingData: () => invoke<RecordingData>('get_recording_data'),
};

export async function listenEvent<T>(event: string, handler: (payload: T) => void): Promise<() => void> {
  if (isTauri) {
    const { listen } = await import('@tauri-apps/api/event');
    const unlisten = await listen<T>(event, (e) => handler(e.payload));
    return unlisten;
  }
  return () => {};
}

export async function openFileDialog(filters: Array<{ name: string; extensions: string[] }>): Promise<string | null> {
  if (isTauri) {
    const { open } = await import('@tauri-apps/plugin-dialog');
    const result = await open({ filters, multiple: false });
    return Array.isArray(result) ? result[0] ?? null : result;
  }
  return new Promise((resolve) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = filters.flatMap(f => f.extensions.map(e => `.${e}`)).join(',');
    input.onchange = () => resolve(input.files?.[0]?.name ?? null);
    input.click();
  });
}
