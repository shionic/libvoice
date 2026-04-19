import "reflect-metadata";
import { NestFactory } from "@nestjs/core";
import { AppModule } from "./app.module";
import { getAppConfig } from "./config";
import { Voxceleb2ImportService } from "./import/voxceleb2-import.service";

interface ImportArguments {
  metadataPath: string;
  speakersPath: string;
  batchSize: number;
  limit?: number;
  dryRun: boolean;
}

function readFlagValue(args: string[], flag: string) {
  const index = args.indexOf(flag);

  if (index === -1) {
    return undefined;
  }

  return args[index + 1];
}

function readIntFlag(args: string[], flag: string) {
  const value = readFlagValue(args, flag);

  if (!value) {
    return undefined;
  }

  const parsed = Number.parseInt(value, 10);

  if (Number.isNaN(parsed)) {
    throw new Error(`Flag ${flag} must be an integer.`);
  }

  return parsed;
}

function printHelp() {
  const config = getAppConfig();

  console.log(`VoiceLib CLI

Commands:
  npm run db:migrate
    Apply SQL migrations to ${config.database.database} on ${config.database.host}:${config.database.port}

  npm run cli -- import-voxceleb2 [options]
    Load VoxCeleb2 speaker metadata and recordings into PostgreSQL.

Options:
  --metadata <path>     Metadata CSV with recording rows
                        Default: ${config.voxceleb2MetadataPath}
  --speakers <path>     VoxCeleb2 speaker metadata CSV
                        Default: ${config.voxceleb2SpeakersPath}
  --batch-size <n>      Insert batch size
                        Default: 500
  --limit <n>           Import only the first n recordings
  --dry-run             Parse files and report counts without writing to the DB
`);
}

function parseImportArguments(args: string[]): ImportArguments {
  const config = getAppConfig();

  return {
    metadataPath:
      readFlagValue(args, "--metadata") ?? config.voxceleb2MetadataPath,
    speakersPath:
      readFlagValue(args, "--speakers") ?? config.voxceleb2SpeakersPath,
    batchSize: readIntFlag(args, "--batch-size") ?? 500,
    limit: readIntFlag(args, "--limit"),
    dryRun: args.includes("--dry-run"),
  };
}

async function main() {
  const [command, ...args] = process.argv.slice(2);

  if (!command || command === "help" || command === "--help") {
    printHelp();
    return;
  }

  if (command !== "import-voxceleb2") {
    printHelp();
    throw new Error(`Unknown command: ${command}`);
  }

  const app = await NestFactory.createApplicationContext(AppModule, {
    logger: ["error", "warn", "log"],
  });

  try {
    const service = app.get(Voxceleb2ImportService);
    await service.importDataset(parseImportArguments(args));
  } finally {
    await app.close();
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
