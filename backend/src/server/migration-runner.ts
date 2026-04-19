import { readFile, readdir } from "node:fs/promises";
import path from "node:path";
import { Pool } from "pg";
import { getDatabasePoolConfig } from "./config";

export async function runMigrations() {
  const pool = new Pool(getDatabasePoolConfig());

  try {
    await pool.query(`
      CREATE TABLE IF NOT EXISTS schema_migrations (
        version TEXT PRIMARY KEY,
        applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      )
    `);

    const migrationsDirectory = path.resolve(__dirname, "migrations");
    const files = (await readdir(migrationsDirectory))
      .filter((fileName) => fileName.endsWith(".sql"))
      .sort();

    const appliedRows = await pool.query<{ version: string }>(
      "SELECT version FROM schema_migrations",
    );
    const applied = new Set(appliedRows.rows.map((row) => row.version));

    for (const fileName of files) {
      if (applied.has(fileName)) {
        continue;
      }

      const client = await pool.connect();

      try {
        const sql = await readFile(
          path.join(migrationsDirectory, fileName),
          "utf8",
        );

        await client.query("BEGIN");
        await client.query(sql);
        await client.query(
          "INSERT INTO schema_migrations (version) VALUES ($1)",
          [fileName],
        );
        await client.query("COMMIT");
        console.log(`Applied migration ${fileName}`);
      } catch (error) {
        await client.query("ROLLBACK");
        throw error;
      } finally {
        client.release();
      }
    }
  } finally {
    await pool.end();
  }
}
