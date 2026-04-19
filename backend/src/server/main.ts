import "reflect-metadata";
import { NestFactory } from "@nestjs/core";
import { AppModule } from "./app.module";
import { getAppConfig } from "./config";

async function bootstrap() {
  const app = await NestFactory.create(AppModule);

  app.enableCors({
    origin: true,
    methods: ["GET", "POST", "OPTIONS"],
    exposedHeaders: ["Accept-Ranges", "Content-Range", "Content-Length"],
  });
  app.setGlobalPrefix("api");

  const config = getAppConfig();
  await app.listen(config.port);
}

bootstrap().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
