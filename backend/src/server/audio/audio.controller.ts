import { access } from "node:fs/promises";
import path from "node:path";
import {
  Controller,
  Get,
  NotFoundException,
  Param,
  ParseIntPipe,
  Res,
} from "@nestjs/common";
import { getAppConfig } from "../config";
import { ComparisonRepository } from "../evaluation/comparison.repository";

interface FileResponse {
  setHeader(name: string, value: string): void;
  sendFile(pathname: string): unknown;
}

@Controller("audio")
export class AudioController {
  constructor(private readonly repository: ComparisonRepository) {}

  @Get(":recordingId")
  async streamRecording(
    @Param("recordingId", ParseIntPipe) recordingId: number,
    @Res() response: FileResponse,
  ) {
    const recording = await this.repository.findRecordingById(recordingId);

    if (!recording) {
      throw new NotFoundException("Recording not found.");
    }

    const baseDirectory = path.resolve(getAppConfig().audioStorageBase);
    const absolutePath = path.resolve(baseDirectory, recording.relativePath);

    if (
      absolutePath !== baseDirectory &&
      !absolutePath.startsWith(`${baseDirectory}${path.sep}`)
    ) {
      throw new NotFoundException("Recording path is invalid.");
    }

    try {
      await access(absolutePath);
    } catch {
      throw new NotFoundException("Recording file does not exist.");
    }

    response.setHeader("Cache-Control", "private, max-age=0, must-revalidate");
    return response.sendFile(absolutePath);
  }
}
