export const ratingPhaseList = ["phase1", "phase2"] as const;
export type RatingPhase = (typeof ratingPhaseList)[number];

export const comparisonSideList = ["left", "right"] as const;
export type ComparisonSide = (typeof comparisonSideList)[number];

export const speakerGenderList = ["male", "female", "unknown"] as const;
export type SpeakerGender = (typeof speakerGenderList)[number];

export const ratingCriteria = [
  {
    id: "genderPresentation",
    dbKey: "gender_presentation",
    label: "Which recording sounds more feminine?",
    shortLabel: "Femininity / Masculinity",
  },
  {
    id: "naturalness",
    dbKey: "naturalness",
    label: "Which recording sounds more natural?",
    shortLabel: "Naturalness",
  },
  {
    id: "attractiveness",
    dbKey: "attractiveness",
    label: "Which recording sounds more attractive?",
    shortLabel: "Attractiveness",
  },
] as const;

export type UiCriterionId = (typeof ratingCriteria)[number]["id"];
export type RatingCriterionDbKey = (typeof ratingCriteria)[number]["dbKey"];

export interface PromptRecordingDto {
  side: ComparisonSide;
  label: string;
  recordingId: number;
  audioPath: string;
}

export interface SessionProgressDto {
  completedPrompts: number;
  completedVotes: number;
}

export interface PhaseEstimateDto {
  completedPrompts: number;
  targetPrompts: number;
  estimatedRemainingPrompts: number;
  progressPercent: number;
  note: string;
}

export interface ComparisonPromptDto {
  id: number;
  phase: RatingPhase;
  selectionReason: string;
  left: PromptRecordingDto;
  right: PromptRecordingDto;
  sessionProgress: SessionProgressDto;
  phaseEstimate: PhaseEstimateDto;
}

export interface NextPromptResponseDto {
  prompt: ComparisonPromptDto;
}

export interface SubmitVoteRequestDto {
  sessionId: string;
  choices: Record<UiCriterionId, ComparisonSide>;
}

export interface SubmitVoteResponseDto {
  savedVotes: number;
  nextPrompt: ComparisonPromptDto;
}

export interface RejectRecordingRequestDto {
  sessionId: string;
  recordingId: number;
  reason?: string;
}

export interface RejectRecordingResponseDto {
  replacedRecordingId: number;
  replacementRecordingId: number;
  prompt: ComparisonPromptDto;
}
