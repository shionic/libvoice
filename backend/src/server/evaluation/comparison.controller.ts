import {
  BadRequestException,
  Body,
  Controller,
  Get,
  Param,
  ParseIntPipe,
  Post,
  Query,
} from "@nestjs/common";
import {
  ratingPhaseList,
  type NextPromptResponseDto,
  type RatingPhase,
  type RejectRecordingRequestDto,
  type RejectRecordingResponseDto,
  type SubmitVoteRequestDto,
  type SubmitVoteResponseDto,
} from "../../shared/contracts";
import { ComparisonService } from "./comparison.service";

function normalizePhase(value: string | undefined): RatingPhase {
  if (!value) {
    return "phase1";
  }

  if (ratingPhaseList.includes(value as RatingPhase)) {
    return value as RatingPhase;
  }

  throw new BadRequestException(`Unsupported phase: ${value}`);
}

@Controller("comparisons")
export class ComparisonController {
  constructor(private readonly comparisonService: ComparisonService) {}

  @Get("next")
  async getNextPrompt(
    @Query("sessionId") sessionId?: string,
    @Query("phase") phase?: string,
  ): Promise<NextPromptResponseDto> {
    if (!sessionId?.trim()) {
      throw new BadRequestException("sessionId is required.");
    }

    return {
      prompt: await this.comparisonService.getNextPrompt(
        sessionId.trim(),
        normalizePhase(phase),
      ),
    };
  }

  @Post(":promptId/vote")
  submitVote(
    @Param("promptId", ParseIntPipe) promptId: number,
    @Body() body: SubmitVoteRequestDto,
  ): Promise<SubmitVoteResponseDto> {
    return this.comparisonService.submitVote(promptId, body);
  }

  @Post(":promptId/reject")
  rejectRecording(
    @Param("promptId", ParseIntPipe) promptId: number,
    @Body() body: RejectRecordingRequestDto,
  ): Promise<RejectRecordingResponseDto> {
    if (!body.sessionId?.trim()) {
      throw new BadRequestException("sessionId is required.");
    }

    return this.comparisonService.rejectRecording(
      promptId,
      body.sessionId.trim(),
      body.recordingId,
      body.reason,
    );
  }
}
