import "reflect-metadata";
import { runMigrations } from "./migration-runner";

async function main() {
  await runMigrations();
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
