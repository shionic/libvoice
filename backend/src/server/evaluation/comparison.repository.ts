import { Injectable } from "@nestjs/common";
import type { PoolClient } from "pg";
import type {
  RatingCriterionDbKey,
  RatingPhase,
  SpeakerGender,
} from "../../shared/contracts";
import { DatabaseService } from "../database.service";
import type {
  PromptRecord,
  PromptReplacement,
  RecordingImportRow,
  RecordingPick,
  ScoreState,
  SessionProgress,
  SpeakerImportRow,
  SpeakerSummary,
  VoteInsertRow,
} from "./types";

type SqlExecutor = {
  query: (
    queryText: string,
    params?: readonly unknown[],
  ) => Promise<{ rows: unknown[] }>;
};

interface PromptRow {
  id: number | string;
  phase: RatingPhase;
  selection_reason: string;
  status: "pending" | "completed" | "abandoned";
  user_session_id: string;
  left_speaker_id: number | string;
  left_speaker_external_id: string;
  left_gender: SpeakerGender;
  left_recording_id: number | string;
  left_relative_path: string;
  right_speaker_id: number | string;
  right_speaker_external_id: string;
  right_gender: SpeakerGender;
  right_recording_id: number | string;
  right_relative_path: string;
  created_at: Date | string;
  completed_at: Date | string | null;
}

interface SpeakerSummaryRow {
  id: number | string;
  external_id: string;
  gender: SpeakerGender;
  active_recordings: number | string;
  gender_rating: number;
  gender_wins: number | string;
  gender_losses: number | string;
  gender_comparisons: number | string;
  naturalness_rating: number;
  naturalness_wins: number | string;
  naturalness_losses: number | string;
  naturalness_comparisons: number | string;
  attractiveness_rating: number;
  attractiveness_wins: number | string;
  attractiveness_losses: number | string;
  attractiveness_comparisons: number | string;
}

interface SessionProgressRow {
  completed_prompts: number | string;
  completed_votes: number | string;
}

interface GenderCountRow {
  gender: SpeakerGender;
  total: number | string;
}

interface PairCountRow {
  speaker_a: number | string;
  speaker_b: number | string;
  total: number | string;
}

interface RecordingRow {
  id: number | string;
  speaker_id: number | string;
  relative_path: string;
}

interface ScoreRow {
  speaker_id: number | string;
  criterion: RatingCriterionDbKey;
  phase: RatingPhase;
  rating: number;
  wins: number | string;
  losses: number | string;
  comparisons: number | string;
}

const promptProjection = `
  SELECT
    cp.id,
    cp.phase,
    cp.selection_reason,
    cp.status,
    cp.user_session_id,
    ls.id AS left_speaker_id,
    ls.external_id AS left_speaker_external_id,
    ls.gender AS left_gender,
    lr.id AS left_recording_id,
    lr.relative_path AS left_relative_path,
    rs.id AS right_speaker_id,
    rs.external_id AS right_speaker_external_id,
    rs.gender AS right_gender,
    rr.id AS right_recording_id,
    rr.relative_path AS right_relative_path,
    cp.created_at,
    cp.completed_at
  FROM comparison_prompts cp
  JOIN speakers ls ON ls.id = cp.left_speaker_id
  JOIN recordings lr ON lr.id = cp.left_recording_id
  JOIN speakers rs ON rs.id = cp.right_speaker_id
  JOIN recordings rr ON rr.id = cp.right_recording_id
`;

function numeric(value: number | string) {
  return typeof value === "number" ? value : Number(value);
}

function toIsoString(value: Date | string) {
  return value instanceof Date ? value.toISOString() : new Date(value).toISOString();
}

function chunk<T>(items: T[], size: number) {
  const batches: T[][] = [];

  for (let index = 0; index < items.length; index += size) {
    batches.push(items.slice(index, index + size));
  }

  return batches;
}

@Injectable()
export class ComparisonRepository {
  constructor(private readonly database: DatabaseService) {}

  private getExecutor(client?: SqlExecutor) {
    return client ?? this.database;
  }

  private mapPromptRow(row: PromptRow): PromptRecord {
    return {
      id: numeric(row.id),
      phase: row.phase,
      selectionReason: row.selection_reason,
      status: row.status,
      userSessionId: row.user_session_id,
      left: {
        speakerId: numeric(row.left_speaker_id),
        speakerExternalId: row.left_speaker_external_id,
        gender: row.left_gender,
        recordingId: numeric(row.left_recording_id),
        relativePath: row.left_relative_path,
      },
      right: {
        speakerId: numeric(row.right_speaker_id),
        speakerExternalId: row.right_speaker_external_id,
        gender: row.right_gender,
        recordingId: numeric(row.right_recording_id),
        relativePath: row.right_relative_path,
      },
      createdAt: toIsoString(row.created_at),
      completedAt: row.completed_at ? toIsoString(row.completed_at) : null,
    };
  }

  async getSessionProgress(sessionId: string): Promise<SessionProgress> {
    const result = await this.database.query<SessionProgressRow>(
      `
        SELECT
          COALESCE(
            (SELECT COUNT(*)::int
             FROM comparison_prompts
             WHERE user_session_id = $1
               AND status = 'completed'),
            0
          ) AS completed_prompts,
          COALESCE(
            (SELECT COUNT(*)::int
             FROM votes
             WHERE user_session_id = $1),
            0
          ) AS completed_votes
      `,
      [sessionId],
    );

    const row = result.rows[0];

    return {
      completedPrompts: numeric(row.completed_prompts),
      completedVotes: numeric(row.completed_votes),
    };
  }

  async countCompletedPrompts(phase: RatingPhase) {
    const result = await this.database.query<{ total: number | string }>(
      `
        SELECT COUNT(*)::int AS total
        FROM comparison_prompts
        WHERE phase = $1
          AND status = 'completed'
      `,
      [phase],
    );

    return numeric(result.rows[0]?.total ?? 0);
  }

  async countActiveSpeakersByGender() {
    const result = await this.database.query<GenderCountRow>(
      `
        SELECT
          s.gender,
          COUNT(*)::int AS total
        FROM speakers s
        WHERE s.gender IN ('male', 'female')
          AND EXISTS (
            SELECT 1
            FROM recordings r
            WHERE r.speaker_id = s.id
              AND r.active = TRUE
          )
        GROUP BY s.gender
      `,
    );

    return {
      male:
        result.rows.find((row) => row.gender === "male")?.total !== undefined
          ? numeric(
              result.rows.find((row) => row.gender === "male")!.total,
            )
          : 0,
      female:
        result.rows.find((row) => row.gender === "female")?.total !== undefined
          ? numeric(
              result.rows.find((row) => row.gender === "female")!.total,
            )
          : 0,
    };
  }

  async countPhaseTwoEligibleSpeakersByGender() {
    const result = await this.database.query<GenderCountRow>(
      `
        SELECT
          s.gender,
          COUNT(*)::int AS total
        FROM speakers s
        JOIN speaker_scores score
          ON score.speaker_id = s.id
         AND score.phase = 'phase1'
         AND score.criterion = 'gender_presentation'
         AND score.comparisons >= 2
        WHERE s.gender IN ('male', 'female')
          AND EXISTS (
            SELECT 1
            FROM recordings r
            WHERE r.speaker_id = s.id
              AND r.active = TRUE
          )
        GROUP BY s.gender
      `,
    );

    return {
      male:
        result.rows.find((row) => row.gender === "male")?.total !== undefined
          ? numeric(
              result.rows.find((row) => row.gender === "male")!.total,
            )
          : 0,
      female:
        result.rows.find((row) => row.gender === "female")?.total !== undefined
          ? numeric(
              result.rows.find((row) => row.gender === "female")!.total,
            )
          : 0,
    };
  }

  async findPendingPrompt(sessionId: string, phase: RatingPhase) {
    const result = await this.database.query<PromptRow>(
      `
        ${promptProjection}
        WHERE cp.user_session_id = $1
          AND cp.phase = $2
          AND cp.status = 'pending'
        ORDER BY cp.created_at DESC
        LIMIT 1
      `,
      [sessionId, phase],
    );

    return result.rows[0] ? this.mapPromptRow(result.rows[0]) : null;
  }

  async findPromptById(
    sessionId: string,
    promptId: number,
    client?: SqlExecutor,
  ) {
    const executor = this.getExecutor(client);
    const result = (await executor.query(
      `
        ${promptProjection}
        WHERE cp.id = $1
          AND cp.user_session_id = $2
        LIMIT 1
      `,
      [promptId, sessionId],
    )) as { rows: PromptRow[] };

    return result.rows[0] ? this.mapPromptRow(result.rows[0]) : null;
  }

  async createPrompt(
    sessionId: string,
    phase: RatingPhase,
    selectionReason: string,
    leftSpeakerId: number,
    rightSpeakerId: number,
    leftRecordingId: number,
    rightRecordingId: number,
  ) {
    const inserted = await this.database.query<{ id: number | string }>(
      `
        INSERT INTO comparison_prompts (
          phase,
          left_speaker_id,
          right_speaker_id,
          left_recording_id,
          right_recording_id,
          selection_reason,
          user_session_id
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING id
      `,
      [
        phase,
        leftSpeakerId,
        rightSpeakerId,
        leftRecordingId,
        rightRecordingId,
        selectionReason,
        sessionId,
      ],
    );

    return this.findPromptById(sessionId, numeric(inserted.rows[0].id));
  }

  async listSpeakerSummaries(
    scorePhase: RatingPhase,
    gender?: SpeakerGender,
  ): Promise<SpeakerSummary[]> {
    const params: unknown[] = [scorePhase];
    const whereClause = gender ? "WHERE s.gender = $2" : "";

    if (gender) {
      params.push(gender);
    }

    const result = await this.database.query<SpeakerSummaryRow>(
      `
        SELECT
          s.id,
          s.external_id,
          s.gender,
          COUNT(r.id)::int AS active_recordings,
          COALESCE(gs.rating, 1500) AS gender_rating,
          COALESCE(gs.wins, 0)::int AS gender_wins,
          COALESCE(gs.losses, 0)::int AS gender_losses,
          COALESCE(gs.comparisons, 0)::int AS gender_comparisons,
          COALESCE(ns.rating, 1500) AS naturalness_rating,
          COALESCE(ns.wins, 0)::int AS naturalness_wins,
          COALESCE(ns.losses, 0)::int AS naturalness_losses,
          COALESCE(ns.comparisons, 0)::int AS naturalness_comparisons,
          COALESCE(ascore.rating, 1500) AS attractiveness_rating,
          COALESCE(ascore.wins, 0)::int AS attractiveness_wins,
          COALESCE(ascore.losses, 0)::int AS attractiveness_losses,
          COALESCE(ascore.comparisons, 0)::int AS attractiveness_comparisons
        FROM speakers s
        JOIN recordings r
          ON r.speaker_id = s.id
         AND r.active = TRUE
        LEFT JOIN speaker_scores gs
          ON gs.speaker_id = s.id
         AND gs.phase = $1
         AND gs.criterion = 'gender_presentation'
        LEFT JOIN speaker_scores ns
          ON ns.speaker_id = s.id
         AND ns.phase = $1
         AND ns.criterion = 'naturalness'
        LEFT JOIN speaker_scores ascore
          ON ascore.speaker_id = s.id
         AND ascore.phase = $1
         AND ascore.criterion = 'attractiveness'
        ${whereClause}
        GROUP BY
          s.id,
          s.external_id,
          s.gender,
          gs.rating,
          gs.wins,
          gs.losses,
          gs.comparisons,
          ns.rating,
          ns.wins,
          ns.losses,
          ns.comparisons,
          ascore.rating,
          ascore.wins,
          ascore.losses,
          ascore.comparisons
        ORDER BY
          (
            COALESCE(gs.comparisons, 0)
            + COALESCE(ns.comparisons, 0)
            + COALESCE(ascore.comparisons, 0)
          ) ASC,
          s.id ASC
      `,
      params,
    );

    return result.rows.map((row) => {
      const scores = {
        gender_presentation: {
          rating: row.gender_rating,
          wins: numeric(row.gender_wins),
          losses: numeric(row.gender_losses),
          comparisons: numeric(row.gender_comparisons),
        },
        naturalness: {
          rating: row.naturalness_rating,
          wins: numeric(row.naturalness_wins),
          losses: numeric(row.naturalness_losses),
          comparisons: numeric(row.naturalness_comparisons),
        },
        attractiveness: {
          rating: row.attractiveness_rating,
          wins: numeric(row.attractiveness_wins),
          losses: numeric(row.attractiveness_losses),
          comparisons: numeric(row.attractiveness_comparisons),
        },
      };

      return {
        id: numeric(row.id),
        externalId: row.external_id,
        gender: row.gender,
        activeRecordings: numeric(row.active_recordings),
        totalComparisons:
          scores.gender_presentation.comparisons +
          scores.naturalness.comparisons +
          scores.attractiveness.comparisons,
        scores,
      };
    });
  }

  async getPairCounts(phase: RatingPhase, speakerIds: number[]) {
    if (speakerIds.length < 2) {
      return new Map<string, number>();
    }

    const result = await this.database.query<PairCountRow>(
      `
        SELECT
          LEAST(left_speaker_id, right_speaker_id) AS speaker_a,
          GREATEST(left_speaker_id, right_speaker_id) AS speaker_b,
          COUNT(*)::int AS total
        FROM comparison_prompts
        WHERE phase = $1
          AND status = 'completed'
          AND left_speaker_id = ANY($2::bigint[])
          AND right_speaker_id = ANY($2::bigint[])
        GROUP BY 1, 2
      `,
      [phase, speakerIds],
    );

    return new Map<string, number>(
      result.rows.map((row) => [
        `${numeric(row.speaker_a)}:${numeric(row.speaker_b)}`,
        numeric(row.total),
      ]),
    );
  }

  async pickRecordingForSpeaker(
    speakerId: number,
    excludedRecordingIds: number[] = [],
  ): Promise<RecordingPick | null> {
    const params: unknown[] = [speakerId];
    let exclusionClause = "";

    if (excludedRecordingIds.length > 0) {
      params.push(excludedRecordingIds);
      exclusionClause = `AND NOT (r.id = ANY($${params.length}::bigint[]))`;
    }

    const result = await this.database.query<RecordingRow>(
      `
        WITH rejection_counts AS (
          SELECT
            rejected_recording_id,
            COUNT(*)::int AS rejection_count
          FROM recording_rejections
          GROUP BY rejected_recording_id
        )
        SELECT
          r.id,
          r.speaker_id,
          r.relative_path
        FROM recordings r
        LEFT JOIN rejection_counts rc
          ON rc.rejected_recording_id = r.id
        WHERE r.speaker_id = $1
          AND r.active = TRUE
          ${exclusionClause}
        ORDER BY
          COALESCE(rc.rejection_count, 0) ASC,
          RANDOM()
        LIMIT 1
      `,
      params,
    );

    if (!result.rows[0]) {
      return null;
    }

    return {
      id: numeric(result.rows[0].id),
      speakerId: numeric(result.rows[0].speaker_id),
      relativePath: result.rows[0].relative_path,
    };
  }

  async findRecordingById(recordingId: number) {
    const result = await this.database.query<RecordingRow>(
      `
        SELECT id, speaker_id, relative_path
        FROM recordings
        WHERE id = $1
        LIMIT 1
      `,
      [recordingId],
    );

    if (!result.rows[0]) {
      return null;
    }

    return {
      id: numeric(result.rows[0].id),
      speakerId: numeric(result.rows[0].speaker_id),
      relativePath: result.rows[0].relative_path,
    };
  }

  async getScoreStates(
    phase: RatingPhase,
    speakerIds: number[],
    client?: SqlExecutor,
  ) {
    const executor = this.getExecutor(client);
    const result = (await executor.query(
      `
        SELECT
          speaker_id,
          criterion,
          phase,
          rating,
          wins,
          losses,
          comparisons
        FROM speaker_scores
        WHERE phase = $1
          AND speaker_id = ANY($2::bigint[])
      `,
      [phase, speakerIds],
    )) as { rows: ScoreRow[] };

    return result.rows.map((row): ScoreState => ({
      speakerId: numeric(row.speaker_id),
      criterion: row.criterion,
      phase: row.phase,
      rating: row.rating,
      wins: numeric(row.wins),
      losses: numeric(row.losses),
      comparisons: numeric(row.comparisons),
    }));
  }

  async insertVotes(votes: VoteInsertRow[], client: SqlExecutor) {
    for (const vote of votes) {
      await client.query(
        `
          INSERT INTO votes (
            prompt_id,
            criterion,
            preferred_recording_id,
            other_recording_id,
            preferred_speaker_id,
            other_speaker_id,
            phase,
            user_session_id
          )
          VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        `,
        [
          vote.promptId,
          vote.criterion,
          vote.preferredRecordingId,
          vote.otherRecordingId,
          vote.preferredSpeakerId,
          vote.otherSpeakerId,
          vote.phase,
          vote.userSessionId,
        ],
      );
    }
  }

  async upsertScoreState(score: ScoreState, client: SqlExecutor) {
    await client.query(
      `
        INSERT INTO speaker_scores (
          speaker_id,
          criterion,
          phase,
          rating,
          wins,
          losses,
          comparisons
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (speaker_id, criterion, phase)
        DO UPDATE SET
          rating = EXCLUDED.rating,
          wins = EXCLUDED.wins,
          losses = EXCLUDED.losses,
          comparisons = EXCLUDED.comparisons,
          updated_at = NOW()
      `,
      [
        score.speakerId,
        score.criterion,
        score.phase,
        score.rating,
        score.wins,
        score.losses,
        score.comparisons,
      ],
    );
  }

  async markPromptCompleted(promptId: number, client: SqlExecutor) {
    await client.query(
      `
        UPDATE comparison_prompts
        SET status = 'completed',
            completed_at = NOW()
        WHERE id = $1
      `,
      [promptId],
    );
  }

  async preparePromptReplacement(
    promptId: number,
    userSessionId: string,
    rejectedRecordingId: number,
    replacement: PromptReplacement,
    reason: string | undefined,
    client: SqlExecutor,
  ) {
    const recordingColumn =
      replacement.side === "left" ? "left_recording_id" : "right_recording_id";
    const replacementCounter =
      replacement.side === "left" ? "replacements_left" : "replacements_right";

    await client.query(
      `
        INSERT INTO recording_rejections (
          prompt_id,
          rejected_recording_id,
          replacement_recording_id,
          user_session_id,
          reason
        )
        VALUES ($1, $2, $3, $4, $5)
      `,
      [
        promptId,
        rejectedRecordingId,
        replacement.replacement.id,
        userSessionId,
        reason ?? null,
      ],
    );

    await client.query(
      `
        UPDATE comparison_prompts
        SET ${recordingColumn} = $1,
            ${replacementCounter} = ${replacementCounter} + 1
        WHERE id = $2
      `,
      [replacement.replacement.id, promptId],
    );
  }

  async upsertSpeakers(rows: SpeakerImportRow[]) {
    for (const batch of chunk(rows, 300)) {
      const values: string[] = [];
      const params: unknown[] = [];

      batch.forEach((row, index) => {
        const offset = index * 6;
        values.push(
          `($${offset + 1}, 'voxceleb2', $${offset + 2}, $${offset + 3}, $${offset + 4}, $${offset + 5}, $${offset + 6}::jsonb)`,
        );
        params.push(
          row.externalId,
          row.gender,
          row.split,
          row.vggface2Id,
          row.tags,
          JSON.stringify(row.metadata),
        );
      });

      await this.database.query(
        `
          INSERT INTO speakers (
            external_id,
            dataset,
            gender,
            split,
            vggface2_id,
            tags,
            metadata
          )
          VALUES ${values.join(", ")}
          ON CONFLICT (external_id)
          DO UPDATE SET
            gender = EXCLUDED.gender,
            split = EXCLUDED.split,
            vggface2_id = EXCLUDED.vggface2_id,
            tags = EXCLUDED.tags,
            metadata = speakers.metadata || EXCLUDED.metadata,
            updated_at = NOW()
        `,
        params,
      );
    }
  }

  async loadSpeakerIdMap(externalIds: string[]) {
    const result = await this.database.query<{
      id: number | string;
      external_id: string;
    }>(
      `
        SELECT id, external_id
        FROM speakers
        WHERE external_id = ANY($1::text[])
      `,
      [externalIds],
    );

    return new Map<string, number>(
      result.rows.map((row) => [row.external_id, numeric(row.id)]),
    );
  }

  async upsertRecordings(rows: RecordingImportRow[]) {
    for (const batch of chunk(rows, 250)) {
      const values: string[] = [];
      const params: unknown[] = [];

      batch.forEach((row, index) => {
        const offset = index * 5;
        values.push(
          `($${offset + 1}, $${offset + 2}, $${offset + 3}, $${offset + 4}, $${offset + 5}::jsonb)`,
        );
        params.push(
          row.speakerId,
          row.relativePath,
          row.fileFormat,
          row.tags,
          JSON.stringify(row.metadata),
        );
      });

      await this.database.query(
        `
          INSERT INTO recordings (
            speaker_id,
            relative_path,
            file_format,
            tags,
            metadata
          )
          VALUES ${values.join(", ")}
          ON CONFLICT (relative_path)
          DO UPDATE SET
            speaker_id = EXCLUDED.speaker_id,
            file_format = EXCLUDED.file_format,
            tags = EXCLUDED.tags,
            metadata = recordings.metadata || EXCLUDED.metadata,
            updated_at = NOW()
        `,
        params,
      );
    }
  }
}
