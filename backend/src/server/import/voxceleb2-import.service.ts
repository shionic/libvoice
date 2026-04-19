import { createReadStream } from "node:fs";
import path from "node:path";
import { Injectable } from "@nestjs/common";
import { parse } from "csv-parse";
import type { SpeakerGender } from "../../shared/contracts";
import { ComparisonRepository } from "../evaluation/comparison.repository";
import type {
  RecordingImportRow,
  SpeakerImportRow,
} from "../evaluation/types";

interface ImportOptions {
  metadataPath: string;
  speakersPath: string;
  batchSize: number;
  limit?: number;
  dryRun: boolean;
}

interface CsvRow {
  [key: string]: string | undefined;
}

function normalizeHeader(header: string) {
  return header
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "");
}

function parseTags(value: string | undefined) {
  if (!value) {
    return [] as string[];
  }

  return value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}

function compactRecord(record: Record<string, unknown>) {
  return Object.fromEntries(
    Object.entries(record).filter(([, value]) => value !== undefined && value !== ""),
  );
}

@Injectable()
export class Voxceleb2ImportService {
  constructor(private readonly repository: ComparisonRepository) {}

  async importDataset(options: ImportOptions) {
    const speakers = await this.readSpeakerMetadata(options.speakersPath);

    if (!options.dryRun) {
      await this.repository.upsertSpeakers([...speakers.values()]);
    }

    const speakerIdMap = options.dryRun
      ? new Map<string, number>()
      : await this.repository.loadSpeakerIdMap([...speakers.keys()]);

    let seenRows = 0;
    let importedRows = 0;
    let skippedRows = 0;
    let buffer: RecordingImportRow[] = [];

    for await (const row of this.streamCsv(options.metadataPath)) {
      const relativePath = row.filepath?.trim();

      if (!relativePath) {
        continue;
      }

      if (options.limit && importedRows >= options.limit) {
        break;
      }

      const speakerExternalId =
        row.author?.trim() ?? this.extractSpeakerIdFromPath(relativePath);
      const speaker = speakers.get(speakerExternalId);

      seenRows += 1;

      if (!speaker) {
        skippedRows += 1;
        continue;
      }

      if (options.dryRun) {
        importedRows += 1;
        continue;
      }

      const speakerId = speakerIdMap.get(speakerExternalId);

      if (!speakerId) {
        skippedRows += 1;
        continue;
      }

      buffer.push({
        speakerId,
        relativePath,
        fileFormat: path.extname(relativePath).replace(".", "") || "m4a",
        tags: parseTags(row.tags),
        metadata: compactRecord({
          author: row.author,
          author_source: row.author_source,
          reliable_quality_rating: row.reliable_quality_rating,
          unreliable_quality_rating: row.unreliable_quality_rating,
          vggface2_id: row.vggface2_id,
          voxceleb2_set: row.voxceleb2_set,
        }),
      });
      importedRows += 1;

      if (buffer.length >= options.batchSize) {
        await this.repository.upsertRecordings(buffer);
        buffer = [];
        console.log(`Imported ${importedRows} recordings...`);
      }
    }

    if (!options.dryRun && buffer.length > 0) {
      await this.repository.upsertRecordings(buffer);
    }

    console.log(`Finished VoxCeleb2 import.
Speakers discovered: ${speakers.size}
Rows parsed: ${seenRows}
Rows imported: ${importedRows}
Rows skipped: ${skippedRows}
Dry run: ${options.dryRun ? "yes" : "no"}`);
  }

  private async readSpeakerMetadata(filePath: string) {
    const speakers = new Map<string, SpeakerImportRow>();

    for await (const row of this.streamCsv(filePath)) {
      const externalId = row.voxceleb2_id?.trim();

      if (!externalId) {
        continue;
      }

      const genderCode = row.gender?.trim().toLowerCase();
      const gender: SpeakerGender =
        genderCode === "f" ? "female" : genderCode === "m" ? "male" : "unknown";
      const split = row.set?.trim() || null;

      speakers.set(externalId, {
        externalId,
        gender,
        split,
        vggface2Id: row.vggface2_id?.trim() || null,
        tags: [gender, split].filter(Boolean) as string[],
        metadata: compactRecord({
          source_file: filePath,
          original_gender: row.gender?.trim(),
          set: split,
        }),
      });
    }

    return speakers;
  }

  private extractSpeakerIdFromPath(relativePath: string) {
    const parts = relativePath.split("/");
    return parts[1] ?? "";
  }

  private async *streamCsv(filePath: string): AsyncGenerator<CsvRow> {
    const parser = createReadStream(filePath).pipe(
      parse({
        bom: true,
        columns: (headers) => headers.map((header) => normalizeHeader(header)),
        skip_empty_lines: true,
        trim: true,
      }),
    );

    for await (const row of parser) {
      yield row as CsvRow;
    }
  }
}
