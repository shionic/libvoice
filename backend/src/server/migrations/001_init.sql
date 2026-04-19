CREATE TYPE speaker_gender AS ENUM ('male', 'female', 'unknown');
CREATE TYPE rating_phase AS ENUM ('phase1', 'phase2');
CREATE TYPE rating_criterion AS ENUM ('gender_presentation', 'naturalness', 'attractiveness');
CREATE TYPE prompt_status AS ENUM ('pending', 'completed', 'abandoned');

CREATE TABLE speakers (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  external_id TEXT NOT NULL UNIQUE,
  dataset TEXT NOT NULL DEFAULT 'voxceleb2',
  gender speaker_gender NOT NULL DEFAULT 'unknown',
  split TEXT,
  vggface2_id TEXT,
  tags TEXT[] NOT NULL DEFAULT '{}',
  metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX speakers_gender_idx ON speakers (gender);
CREATE INDEX speakers_dataset_idx ON speakers (dataset);

CREATE TABLE recordings (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  speaker_id BIGINT NOT NULL REFERENCES speakers(id) ON DELETE CASCADE,
  relative_path TEXT NOT NULL UNIQUE,
  file_format TEXT NOT NULL DEFAULT 'm4a',
  active BOOLEAN NOT NULL DEFAULT TRUE,
  tags TEXT[] NOT NULL DEFAULT '{}',
  metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX recordings_speaker_active_idx ON recordings (speaker_id, active);
CREATE INDEX recordings_active_idx ON recordings (active);
CREATE INDEX recordings_tags_idx ON recordings USING GIN (tags);

CREATE TABLE comparison_prompts (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  phase rating_phase NOT NULL,
  left_speaker_id BIGINT NOT NULL REFERENCES speakers(id),
  right_speaker_id BIGINT NOT NULL REFERENCES speakers(id),
  left_recording_id BIGINT NOT NULL REFERENCES recordings(id),
  right_recording_id BIGINT NOT NULL REFERENCES recordings(id),
  selection_reason TEXT NOT NULL,
  user_session_id TEXT NOT NULL,
  status prompt_status NOT NULL DEFAULT 'pending',
  replacements_left INTEGER NOT NULL DEFAULT 0,
  replacements_right INTEGER NOT NULL DEFAULT 0,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  completed_at TIMESTAMPTZ
);

CREATE INDEX comparison_prompts_session_status_idx
  ON comparison_prompts (user_session_id, status, created_at DESC);
CREATE INDEX comparison_prompts_phase_status_idx
  ON comparison_prompts (phase, status, created_at DESC);
CREATE INDEX comparison_prompts_pair_norm_idx
  ON comparison_prompts (
    phase,
    LEAST(left_speaker_id, right_speaker_id),
    GREATEST(left_speaker_id, right_speaker_id)
  );

CREATE TABLE votes (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  prompt_id BIGINT NOT NULL REFERENCES comparison_prompts(id) ON DELETE CASCADE,
  criterion rating_criterion NOT NULL,
  preferred_recording_id BIGINT NOT NULL REFERENCES recordings(id),
  other_recording_id BIGINT NOT NULL REFERENCES recordings(id),
  preferred_speaker_id BIGINT NOT NULL REFERENCES speakers(id),
  other_speaker_id BIGINT NOT NULL REFERENCES speakers(id),
  phase rating_phase NOT NULL,
  user_session_id TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  CONSTRAINT votes_prompt_criterion_unique UNIQUE (prompt_id, criterion),
  CONSTRAINT votes_distinct_recordings_check CHECK (preferred_recording_id <> other_recording_id),
  CONSTRAINT votes_distinct_speakers_check CHECK (preferred_speaker_id <> other_speaker_id)
);

CREATE INDEX votes_phase_criterion_idx ON votes (phase, criterion, created_at DESC);
CREATE INDEX votes_session_idx ON votes (user_session_id, created_at DESC);
CREATE INDEX votes_preferred_speaker_idx ON votes (preferred_speaker_id, criterion);
CREATE INDEX votes_other_speaker_idx ON votes (other_speaker_id, criterion);

CREATE TABLE recording_rejections (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  prompt_id BIGINT NOT NULL REFERENCES comparison_prompts(id) ON DELETE CASCADE,
  rejected_recording_id BIGINT NOT NULL REFERENCES recordings(id),
  replacement_recording_id BIGINT REFERENCES recordings(id),
  user_session_id TEXT NOT NULL,
  reason TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX recording_rejections_recording_idx
  ON recording_rejections (rejected_recording_id, created_at DESC);
CREATE INDEX recording_rejections_prompt_idx
  ON recording_rejections (prompt_id, created_at DESC);

CREATE TABLE speaker_scores (
  speaker_id BIGINT NOT NULL REFERENCES speakers(id) ON DELETE CASCADE,
  criterion rating_criterion NOT NULL,
  phase rating_phase NOT NULL,
  rating DOUBLE PRECISION NOT NULL DEFAULT 1500,
  wins INTEGER NOT NULL DEFAULT 0,
  losses INTEGER NOT NULL DEFAULT 0,
  comparisons INTEGER NOT NULL DEFAULT 0,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (speaker_id, criterion, phase)
);

CREATE INDEX speaker_scores_phase_criterion_rating_idx
  ON speaker_scores (phase, criterion, rating);
CREATE INDEX speaker_scores_phase_criterion_comparisons_idx
  ON speaker_scores (phase, criterion, comparisons);
