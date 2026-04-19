import type { PoolConfig } from "pg";

export interface AppConfig {
  port: number;
  audioStorageBase: string;
  voxceleb2MetadataPath: string;
  voxceleb2SpeakersPath: string;
  database: PoolConfig;
}

let cachedConfig: AppConfig | undefined;
let envLoaded = false;

function ensureEnvLoaded() {
  if (envLoaded) {
    return;
  }

  const runtimeProcess = process as typeof process & {
    loadEnvFile?: (path?: string) => void;
  };

  if (typeof runtimeProcess.loadEnvFile === "function") {
    for (const fileName of [".env", ".env.local"]) {
      try {
        runtimeProcess.loadEnvFile(fileName);
      } catch {
        // Ignore missing env files and fall back to shell variables/defaults.
      }
    }
  }

  envLoaded = true;
}

function readString(name: string, fallback: string) {
  const value = process.env[name];
  return value && value.trim().length > 0 ? value.trim() : fallback;
}

function readInt(name: string, fallback: number) {
  const rawValue = process.env[name];

  if (!rawValue) {
    return fallback;
  }

  const parsed = Number.parseInt(rawValue, 10);

  if (Number.isNaN(parsed)) {
    throw new Error(`Environment variable ${name} must be an integer.`);
  }

  return parsed;
}

export function getAppConfig(): AppConfig {
  ensureEnvLoaded();

  if (!cachedConfig) {
    cachedConfig = {
      port: readInt("PORT", 3001),
      audioStorageBase: readString(
        "AUDIO_STORAGE_BASE",
        "/media/data/experiment/voxeleb2",
      ),
      voxceleb2MetadataPath: readString(
        "VOXCELEB2_METADATA_PATH",
        "/media/data/experiment/voxeleb2/metadata.csv",
      ),
      voxceleb2SpeakersPath: readString(
        "VOXCELEB2_SPEAKERS_PATH",
        "/media/data/experiment/voxeleb2/vox2_meta.csv",
      ),
      database: {
        host: readString("DATABASE_HOST", "127.0.0.1"),
        port: readInt("DATABASE_PORT", 5432),
        database: readString("DATABASE_NAME", "libvoice"),
        user: readString("DATABASE_USER", "libvoice"),
        password: readString("DATABASE_PASSWORD", "1111"),
      },
    };
  }

  return cachedConfig;
}

export function getDatabasePoolConfig(): PoolConfig {
  return getAppConfig().database;
}
