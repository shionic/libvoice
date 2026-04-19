import {
  BadRequestException,
  ConflictException,
  Injectable,
  NotFoundException,
} from "@nestjs/common";
import {
  ratingCriteria,
  type ComparisonPromptDto,
  type ComparisonSide,
  type PhaseEstimateDto,
  type RatingCriterionDbKey,
  type RatingPhase,
  type RejectRecordingResponseDto,
  type SessionProgressDto,
  type SpeakerGender,
  type SubmitVoteRequestDto,
  type SubmitVoteResponseDto,
  type UiCriterionId,
} from "../../shared/contracts";
import { DatabaseService } from "../database.service";
import { ComparisonRepository } from "./comparison.repository";
import type {
  PairCandidate,
  PromptRecord,
  PromptReplacement,
  ScoreState,
  SpeakerSummary,
  VoteInsertRow,
} from "./types";

const selectionReasonLabels: Record<string, string> = {
  coverage_gap: "Coverage-first same-gender pair",
  frontier_same_gender: "Same-gender frontier pair",
  cross_gender_frontier: "Cross-gender frontier pair",
};

const phaseOneTargetComparisonsPerSpeaker = 8;
const phaseTwoTargetComparisonsPerFrontierSpeaker = 6;

function randomItem<T>(items: T[]) {
  return items[Math.floor(Math.random() * items.length)];
}

function sortPairKey(leftSpeakerId: number, rightSpeakerId: number) {
  const sorted = [leftSpeakerId, rightSpeakerId].sort((a, b) => a - b);
  return `${sorted[0]}:${sorted[1]}`;
}

function totalCoverage(speaker: SpeakerSummary) {
  return speaker.totalComparisons;
}

function criterionDistance(
  leftSpeaker: SpeakerSummary,
  rightSpeaker: SpeakerSummary,
  criterion: RatingCriterionDbKey,
) {
  return (
    Math.abs(
      leftSpeaker.scores[criterion].rating - rightSpeaker.scores[criterion].rating,
    ) / 120
  );
}

function vectorDistance(leftSpeaker: SpeakerSummary, rightSpeaker: SpeakerSummary) {
  return (
    criterionDistance(leftSpeaker, rightSpeaker, "gender_presentation") * 1.3 +
    criterionDistance(leftSpeaker, rightSpeaker, "naturalness") +
    criterionDistance(leftSpeaker, rightSpeaker, "attractiveness")
  );
}

function eloUpdate(winner: ScoreState, loser: ScoreState) {
  const expectedWinner =
    1 / (1 + 10 ** ((loser.rating - winner.rating) / 400));
  const expectedLoser = 1 - expectedWinner;
  const winnerK = Math.max(10, 40 / Math.sqrt(winner.comparisons + 1));
  const loserK = Math.max(10, 40 / Math.sqrt(loser.comparisons + 1));

  return {
    winner: {
      ...winner,
      rating: winner.rating + winnerK * (1 - expectedWinner),
      wins: winner.wins + 1,
      comparisons: winner.comparisons + 1,
    },
    loser: {
      ...loser,
      rating: loser.rating + loserK * (0 - expectedLoser),
      losses: loser.losses + 1,
      comparisons: loser.comparisons + 1,
    },
  };
}

@Injectable()
export class ComparisonService {
  constructor(
    private readonly repository: ComparisonRepository,
    private readonly database: DatabaseService,
  ) {}

  async getNextPrompt(
    sessionId: string,
    phase: RatingPhase,
  ): Promise<ComparisonPromptDto> {
    if (!sessionId.trim()) {
      throw new BadRequestException("sessionId is required.");
    }

    const pendingPrompt = await this.repository.findPendingPrompt(sessionId, phase);

    if (pendingPrompt) {
      const [sessionProgress, phaseEstimate] = await Promise.all([
        this.repository.getSessionProgress(sessionId),
        this.getPhaseEstimate(phase),
      ]);

      return this.toDto(
        pendingPrompt,
        sessionProgress,
        phaseEstimate,
      );
    }

    const candidate =
      phase === "phase2"
        ? await this.buildPhaseTwoCandidate()
        : await this.buildPhaseOneCandidate();

    if (!candidate) {
      throw new NotFoundException(
        phase === "phase2"
          ? "Phase 2 needs more phase 1 speaker coverage before frontier cross-gender pairs can be served."
          : "No eligible recordings are available. Run the migrations and import the dataset first.",
      );
    }

    const prompt = await this.repository.createPrompt(
      sessionId,
      candidate.phase,
      candidate.selectionReason,
      candidate.leftSpeaker.id,
      candidate.rightSpeaker.id,
      candidate.leftRecording.id,
      candidate.rightRecording.id,
    );

    if (!prompt) {
      throw new NotFoundException("Failed to create the next comparison prompt.");
    }

    const [sessionProgress, phaseEstimate] = await Promise.all([
      this.repository.getSessionProgress(sessionId),
      this.getPhaseEstimate(phase),
    ]);

    return this.toDto(prompt, sessionProgress, phaseEstimate);
  }

  async submitVote(
    promptId: number,
    body: SubmitVoteRequestDto,
  ): Promise<SubmitVoteResponseDto> {
    const sessionId = body.sessionId?.trim();

    if (!sessionId) {
      throw new BadRequestException("sessionId is required.");
    }

    const prompt = await this.repository.findPromptById(sessionId, promptId);

    if (!prompt) {
      throw new NotFoundException("Prompt not found for this session.");
    }

    if (prompt.status !== "pending") {
      throw new ConflictException("This prompt has already been completed.");
    }

    const votes = this.buildVotes(prompt, body);

    await this.database.withTransaction(async (client) => {
      const livePrompt = await this.repository.findPromptById(
        sessionId,
        promptId,
        client,
      );

      if (!livePrompt || livePrompt.status !== "pending") {
        throw new ConflictException("The prompt is no longer available.");
      }

      await this.repository.insertVotes(votes, client);

      const currentScores = await this.repository.getScoreStates(
        livePrompt.phase,
        [livePrompt.left.speakerId, livePrompt.right.speakerId],
        client,
      );

      const updatedScores = this.applyVotesToScores(currentScores, votes);

      for (const score of updatedScores.values()) {
        await this.repository.upsertScoreState(score, client);
      }

      await this.repository.markPromptCompleted(promptId, client);
    });

    return {
      savedVotes: votes.length,
      nextPrompt: await this.getNextPrompt(sessionId, prompt.phase),
    };
  }

  async rejectRecording(
    promptId: number,
    sessionId: string,
    recordingId: number,
    reason?: string,
  ): Promise<RejectRecordingResponseDto> {
    const prompt = await this.repository.findPromptById(sessionId, promptId);

    if (!prompt) {
      throw new NotFoundException("Prompt not found for this session.");
    }

    if (prompt.status !== "pending") {
      throw new ConflictException("Completed prompts cannot be edited.");
    }

    const replacement = await this.prepareReplacement(prompt, recordingId);

    await this.database.withTransaction(async (client) => {
      await this.repository.preparePromptReplacement(
        promptId,
        sessionId,
        recordingId,
        replacement,
        reason,
        client,
      );
    });

    const updatedPrompt = await this.repository.findPromptById(sessionId, promptId);

    if (!updatedPrompt) {
      throw new NotFoundException("Updated prompt could not be loaded.");
    }

    return {
      replacedRecordingId: recordingId,
      replacementRecordingId: replacement.replacement.id,
      prompt: this.toDto(updatedPrompt, ...(await Promise.all([
        this.repository.getSessionProgress(sessionId),
        this.getPhaseEstimate(updatedPrompt.phase),
      ]))),
    };
  }

  private async getPhaseEstimate(phase: RatingPhase): Promise<PhaseEstimateDto> {
    const completedPrompts = await this.repository.countCompletedPrompts(phase);

    if (phase === "phase1") {
      const activeSpeakers = await this.repository.countActiveSpeakersByGender();
      const maleTarget =
        activeSpeakers.male >= 2
          ? Math.ceil((activeSpeakers.male * phaseOneTargetComparisonsPerSpeaker) / 2)
          : 0;
      const femaleTarget =
        activeSpeakers.female >= 2
          ? Math.ceil(
              (activeSpeakers.female * phaseOneTargetComparisonsPerSpeaker) / 2,
            )
          : 0;
      const targetPrompts = maleTarget + femaleTarget;

      return this.buildPhaseEstimateDto(
        completedPrompts,
        targetPrompts,
        `Assumes about ${phaseOneTargetComparisonsPerSpeaker} same-gender comparisons per speaker across ${activeSpeakers.female} female and ${activeSpeakers.male} male speakers with active recordings.`,
      );
    }

    const eligibleSpeakers =
      await this.repository.countPhaseTwoEligibleSpeakersByGender();
    const femaleFrontierCount = this.computeFrontierSize(eligibleSpeakers.female);
    const maleFrontierCount = this.computeFrontierSize(eligibleSpeakers.male);

    if (femaleFrontierCount < 2 || maleFrontierCount < 2) {
      return this.buildPhaseEstimateDto(
        completedPrompts,
        0,
        "Phase 2 unlocks after enough phase 1 femininity comparisons exist to define both male and female frontier groups.",
      );
    }

    const targetPrompts =
      Math.max(femaleFrontierCount, maleFrontierCount) *
      phaseTwoTargetComparisonsPerFrontierSpeaker;

    return this.buildPhaseEstimateDto(
      completedPrompts,
      targetPrompts,
      `Assumes about ${phaseTwoTargetComparisonsPerFrontierSpeaker} cross-gender frontier comparisons per speaker across ${femaleFrontierCount} female and ${maleFrontierCount} male boundary speakers.`,
    );
  }

  private computeFrontierSize(eligibleSpeakerCount: number) {
    if (eligibleSpeakerCount <= 0) {
      return 0;
    }

    return Math.min(
      eligibleSpeakerCount,
      Math.max(2, Math.ceil(eligibleSpeakerCount * 0.2)),
    );
  }

  private buildPhaseEstimateDto(
    completedPrompts: number,
    targetPrompts: number,
    note: string,
  ): PhaseEstimateDto {
    const estimatedRemainingPrompts = Math.max(0, targetPrompts - completedPrompts);
    const progressPercent =
      targetPrompts > 0
        ? Math.min(100, Math.round((completedPrompts / targetPrompts) * 100))
        : 0;

    return {
      completedPrompts,
      targetPrompts,
      estimatedRemainingPrompts,
      progressPercent,
      note,
    };
  }

  private async prepareReplacement(
    prompt: PromptRecord,
    recordingId: number,
  ): Promise<PromptReplacement> {
    if (recordingId === prompt.left.recordingId) {
      const replacement = await this.repository.pickRecordingForSpeaker(
        prompt.left.speakerId,
        [prompt.left.recordingId, prompt.right.recordingId],
      );

      if (!replacement) {
        throw new ConflictException(
          "No alternative recording is available for the left speaker.",
        );
      }

      return {
        side: "left",
        speakerId: prompt.left.speakerId,
        replacement,
      };
    }

    if (recordingId === prompt.right.recordingId) {
      const replacement = await this.repository.pickRecordingForSpeaker(
        prompt.right.speakerId,
        [prompt.left.recordingId, prompt.right.recordingId],
      );

      if (!replacement) {
        throw new ConflictException(
          "No alternative recording is available for the right speaker.",
        );
      }

      return {
        side: "right",
        speakerId: prompt.right.speakerId,
        replacement,
      };
    }

    throw new BadRequestException("The rejected recording is not part of this prompt.");
  }

  private buildVotes(
    prompt: PromptRecord,
    body: SubmitVoteRequestDto,
  ): VoteInsertRow[] {
    const missingCriteria = ratingCriteria.filter(
      (criterion) => !body.choices?.[criterion.id],
    );

    if (missingCriteria.length > 0) {
      throw new BadRequestException("All three criteria must be answered.");
    }

    return ratingCriteria.map((criterion) => {
      const side = body.choices[criterion.id];
      const preferred =
        side === "left"
          ? {
              recordingId: prompt.left.recordingId,
              speakerId: prompt.left.speakerId,
            }
          : {
              recordingId: prompt.right.recordingId,
              speakerId: prompt.right.speakerId,
            };
      const other =
        side === "left"
          ? {
              recordingId: prompt.right.recordingId,
              speakerId: prompt.right.speakerId,
            }
          : {
              recordingId: prompt.left.recordingId,
              speakerId: prompt.left.speakerId,
            };

      return {
        promptId: prompt.id,
        criterion: criterion.dbKey,
        preferredRecordingId: preferred.recordingId,
        otherRecordingId: other.recordingId,
        preferredSpeakerId: preferred.speakerId,
        otherSpeakerId: other.speakerId,
        phase: prompt.phase,
        userSessionId: body.sessionId,
      };
    });
  }

  private applyVotesToScores(existingScores: ScoreState[], votes: VoteInsertRow[]) {
    const scoreMap = new Map<string, ScoreState>();

    for (const score of existingScores) {
      scoreMap.set(this.scoreKey(score.speakerId, score.criterion), score);
    }

    for (const vote of votes) {
      const winnerKey = this.scoreKey(vote.preferredSpeakerId, vote.criterion);
      const loserKey = this.scoreKey(vote.otherSpeakerId, vote.criterion);
      const winnerScore =
        scoreMap.get(winnerKey) ??
        this.createDefaultScore(vote.preferredSpeakerId, vote.criterion, vote.phase);
      const loserScore =
        scoreMap.get(loserKey) ??
        this.createDefaultScore(vote.otherSpeakerId, vote.criterion, vote.phase);
      const updated = eloUpdate(winnerScore, loserScore);

      scoreMap.set(winnerKey, updated.winner);
      scoreMap.set(loserKey, updated.loser);
    }

    return scoreMap;
  }

  private createDefaultScore(
    speakerId: number,
    criterion: RatingCriterionDbKey,
    phase: RatingPhase,
  ): ScoreState {
    return {
      speakerId,
      criterion,
      phase,
      rating: 1500,
      wins: 0,
      losses: 0,
      comparisons: 0,
    };
  }

  private scoreKey(speakerId: number, criterion: RatingCriterionDbKey) {
    return `${speakerId}:${criterion}`;
  }

  private async buildPhaseOneCandidate(): Promise<PairCandidate | null> {
    const allSpeakers = (await this.repository.listSpeakerSummaries("phase1")).filter(
      (speaker) => speaker.gender !== "unknown" && speaker.activeRecordings > 0,
    );

    const buckets = (
      ["female", "male"] as const satisfies readonly SpeakerGender[]
    )
      .map((gender) =>
        allSpeakers.filter((speaker) => speaker.gender === gender).slice(0, 120),
      )
      .filter((bucket) => bucket.length >= 2)
      .sort(
        (leftBucket, rightBucket) =>
          totalCoverage(leftBucket[0]) - totalCoverage(rightBucket[0]),
      );

    for (const bucket of buckets) {
      const pairCounts = await this.repository.getPairCounts(
        "phase1",
        bucket.map((speaker) => speaker.id),
      );
      const candidate = await this.selectSameGenderPair(bucket, pairCounts, "phase1");

      if (candidate) {
        return candidate;
      }
    }

    return null;
  }

  private async buildPhaseTwoCandidate(): Promise<PairCandidate | null> {
    const phaseOneSpeakers = await this.repository.listSpeakerSummaries("phase1");
    const femaleBoundary = phaseOneSpeakers
      .filter(
        (speaker) =>
          speaker.gender === "female" &&
          speaker.scores.gender_presentation.comparisons >= 2,
      )
      .sort(
        (leftSpeaker, rightSpeaker) =>
          leftSpeaker.scores.gender_presentation.rating -
          rightSpeaker.scores.gender_presentation.rating,
      );
    const maleBoundary = phaseOneSpeakers
      .filter(
        (speaker) =>
          speaker.gender === "male" &&
          speaker.scores.gender_presentation.comparisons >= 2,
      )
      .sort(
        (leftSpeaker, rightSpeaker) =>
          rightSpeaker.scores.gender_presentation.rating -
          leftSpeaker.scores.gender_presentation.rating,
      );

    const femaleFrontier = femaleBoundary.slice(
      0,
      Math.max(2, Math.ceil(femaleBoundary.length * 0.2)),
    );
    const maleFrontier = maleBoundary.slice(
      0,
      Math.max(2, Math.ceil(maleBoundary.length * 0.2)),
    );

    if (femaleFrontier.length < 2 || maleFrontier.length < 2) {
      return null;
    }

    const pairCounts = await this.repository.getPairCounts("phase2", [
      ...femaleFrontier.map((speaker) => speaker.id),
      ...maleFrontier.map((speaker) => speaker.id),
    ]);

    let bestPair:
      | {
          leftSpeaker: SpeakerSummary;
          rightSpeaker: SpeakerSummary;
          score: number;
        }
      | undefined;

    for (const leftSpeaker of femaleFrontier.slice(0, 30)) {
      for (const rightSpeaker of maleFrontier.slice(0, 30)) {
        const pairCount =
          pairCounts.get(sortPairKey(leftSpeaker.id, rightSpeaker.id)) ?? 0;
        const genderCloseness =
          1 /
          (1 +
            Math.abs(
              leftSpeaker.scores.gender_presentation.rating -
                rightSpeaker.scores.gender_presentation.rating,
            ) /
              55);
        const overallCloseness = 1 / (1 + vectorDistance(leftSpeaker, rightSpeaker));
        const exploration =
          1 / (1 + pairCount) +
          1 / (1 + (leftSpeaker.totalComparisons + rightSpeaker.totalComparisons) / 6);
        const score =
          genderCloseness * 0.55 + overallCloseness * 0.2 + exploration * 0.25;

        if (!bestPair || score > bestPair.score) {
          bestPair = {
            leftSpeaker,
            rightSpeaker,
            score,
          };
        }
      }
    }

    if (!bestPair) {
      return null;
    }

    const leftRecording = await this.repository.pickRecordingForSpeaker(
      bestPair.leftSpeaker.id,
    );
    const rightRecording = await this.repository.pickRecordingForSpeaker(
      bestPair.rightSpeaker.id,
    );

    if (!leftRecording || !rightRecording) {
      return null;
    }

    return {
      phase: "phase2",
      selectionReason: "cross_gender_frontier",
      leftSpeaker: bestPair.leftSpeaker,
      rightSpeaker: bestPair.rightSpeaker,
      leftRecording,
      rightRecording,
    };
  }

  private async selectSameGenderPair(
    speakers: SpeakerSummary[],
    pairCounts: Map<string, number>,
    phase: RatingPhase,
  ): Promise<PairCandidate | null> {
    const baseCandidates = speakers
      .slice(0, 18)
      .sort(
        (leftSpeaker, rightSpeaker) =>
          leftSpeaker.totalComparisons - rightSpeaker.totalComparisons,
      )
      .slice(0, 8);

    let bestPair:
      | {
          leftSpeaker: SpeakerSummary;
          rightSpeaker: SpeakerSummary;
          score: number;
          selectionReason: string;
        }
      | undefined;

    for (const leftSpeaker of baseCandidates) {
      for (const rightSpeaker of speakers) {
        if (leftSpeaker.id === rightSpeaker.id) {
          continue;
        }

        const pairCount =
          pairCounts.get(sortPairKey(leftSpeaker.id, rightSpeaker.id)) ?? 0;
        const closeness = 1 / (1 + vectorDistance(leftSpeaker, rightSpeaker));
        const exploration =
          1 / (1 + pairCount) +
          1 / (1 + Math.abs(leftSpeaker.totalComparisons - rightSpeaker.totalComparisons));
        const lowCoverageBoost =
          1 /
          (1 +
            (leftSpeaker.totalComparisons + rightSpeaker.totalComparisons) / 4);
        const score =
          closeness * 0.5 + exploration * 0.35 + lowCoverageBoost * 0.15;
        const selectionReason =
          vectorDistance(leftSpeaker, rightSpeaker) < 1.2
            ? "frontier_same_gender"
            : "coverage_gap";

        if (!bestPair || score > bestPair.score) {
          bestPair = {
            leftSpeaker,
            rightSpeaker,
            score,
            selectionReason,
          };
        }
      }
    }

    if (!bestPair) {
      return null;
    }

    const leftRecording = await this.repository.pickRecordingForSpeaker(
      bestPair.leftSpeaker.id,
    );
    const rightRecording = await this.repository.pickRecordingForSpeaker(
      bestPair.rightSpeaker.id,
    );

    if (!leftRecording || !rightRecording) {
      return null;
    }

    const [leftSpeaker, rightSpeaker] =
      Math.random() > 0.5
        ? [bestPair.leftSpeaker, bestPair.rightSpeaker]
        : [bestPair.rightSpeaker, bestPair.leftSpeaker];
    const [leftPick, rightPick] =
      leftSpeaker.id === bestPair.leftSpeaker.id
        ? [leftRecording, rightRecording]
        : [rightRecording, leftRecording];

    return {
      phase,
      selectionReason: bestPair.selectionReason,
      leftSpeaker,
      rightSpeaker,
      leftRecording: leftPick,
      rightRecording: rightPick,
    };
  }

  private toDto(
    prompt: PromptRecord,
    sessionProgress: SessionProgressDto,
    phaseEstimate: PhaseEstimateDto,
  ): ComparisonPromptDto {
    return {
      id: prompt.id,
      phase: prompt.phase,
      selectionReason:
        selectionReasonLabels[prompt.selectionReason] ?? prompt.selectionReason,
      sessionProgress,
      phaseEstimate,
      left: {
        side: "left",
        label: "Recording A",
        recordingId: prompt.left.recordingId,
        audioPath: `/api/audio/${prompt.left.recordingId}`,
      },
      right: {
        side: "right",
        label: "Recording B",
        recordingId: prompt.right.recordingId,
        audioPath: `/api/audio/${prompt.right.recordingId}`,
      },
    };
  }
}
