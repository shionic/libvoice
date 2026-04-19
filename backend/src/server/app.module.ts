import { Module } from "@nestjs/common";
import { ConfigModule } from "@nestjs/config";
import { AudioController } from "./audio/audio.controller";
import { DatabaseService } from "./database.service";
import { ComparisonController } from "./evaluation/comparison.controller";
import { ComparisonRepository } from "./evaluation/comparison.repository";
import { ComparisonService } from "./evaluation/comparison.service";
import { HealthController } from "./health.controller";
import { Voxceleb2ImportService } from "./import/voxceleb2-import.service";

@Module({
  imports: [ConfigModule.forRoot({ isGlobal: true })],
  controllers: [AudioController, ComparisonController, HealthController],
  providers: [
    DatabaseService,
    ComparisonRepository,
    ComparisonService,
    Voxceleb2ImportService,
  ],
})
export class AppModule {}
