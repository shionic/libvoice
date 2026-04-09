class MonoChunkProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    const processorOptions = options.processorOptions ?? {};
    const chunkDurationMs = processorOptions.chunkDurationMs ?? 100;

    this.samplesPerChunk = Math.max(
      128,
      Math.round((sampleRate * chunkDurationMs) / 1000),
    );
    this.pending = new Float32Array(this.samplesPerChunk * 2);
    this.pendingLength = 0;
  }

  process(inputs) {
    const input = inputs[0];
    const mono = input?.[0];

    if (!mono || mono.length === 0) {
      return true;
    }

    this.append(mono);

    while (this.pendingLength >= this.samplesPerChunk) {
      const chunk = this.pending.slice(0, this.samplesPerChunk);
      this.port.postMessage(chunk, [chunk.buffer]);
      this.pending.copyWithin(0, this.samplesPerChunk, this.pendingLength);
      this.pendingLength -= this.samplesPerChunk;
    }

    return true;
  }

  append(samples) {
    if (this.pendingLength + samples.length > this.pending.length) {
      const next = new Float32Array(
        Math.max(this.pending.length * 2, this.pendingLength + samples.length),
      );
      next.set(this.pending.subarray(0, this.pendingLength));
      this.pending = next;
    }

    this.pending.set(samples, this.pendingLength);
    this.pendingLength += samples.length;
  }
}

registerProcessor("mono-chunk-processor", MonoChunkProcessor);
