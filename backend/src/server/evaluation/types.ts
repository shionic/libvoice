import type {
  ComparisonSide,
  RatingCriterionDbKey,
  RatingPhase,
  SpeakerGender,
} from "../../shared/contracts";

export interface CriterionScore {
  rating: number;
  wins: number;
  losses: number;
  comparisons: number;
}

export interface SpeakerSummary {
  id: number;
  externalId: string;
  gender: SpeakerGender;
  activeRecordings: number;
  totalComparisons: number;
  scores: Record<RatingCriterionDbKey, CriterionScore>;
}

export interface RecordingPick {
  id: number;
  speakerId: number;
  relativePath: string;
}

export interface PromptSideRecord {
  speakerId: number;
  speakerExternalId: string;
  gender: SpeakerGender;
  recordingId: number;
  relativePath: string;
}

export interface PromptRecord {
  id: number;
  phase: RatingPhase;
  selectionReason: string;
  status: "pending" | "completed" | "abandoned";
  userSessionId: string;
  left: PromptSideRecord;
  right: PromptSideRecord;
  createdAt: string;
  completedAt: string | null;
}

export interface SessionProgress {
  completedPrompts: number;
  completedVotes: number;
}

export interface PairCandidate {
  phase: RatingPhase;
  selectionReason: string;
  leftSpeaker: SpeakerSummary;
  rightSpeaker: SpeakerSummary;
  leftRecording: RecordingPick;
  rightRecording: RecordingPick;
}

export interface VoteInsertRow {
  promptId: number;
  criterion: RatingCriterionDbKey;
  preferredRecordingId: number;
  otherRecordingId: number;
  preferredSpeakerId: number;
  otherSpeakerId: number;
  phase: RatingPhase;
  userSessionId: string;
}

export interface ScoreState {
  speakerId: number;
  criterion: RatingCriterionDbKey;
  phase: RatingPhase;
  rating: number;
  wins: number;
  losses: number;
  comparisons: number;
}

export interface SpeakerImportRow {
  externalId: string;
  gender: SpeakerGender;
  split: string | null;
  vggface2Id: string | null;
  tags: string[];
  metadata: Record<string, unknown>;
}

export interface RecordingImportRow {
  speakerId: number;
  relativePath: string;
  fileFormat: string;
  tags: string[];
  metadata: Record<string, unknown>;
}

export interface PromptReplacement {
  side: ComparisonSide;
  speakerId: number;
  replacement: RecordingPick;
}
