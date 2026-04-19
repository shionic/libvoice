# VoiceLib Rating Service

NestJS API plus a Next.js rating client for pairwise voice evaluation on three axes:

- Femininity / Masculinity
- Naturalness
- Attractiveness

The system stores pairwise preferences in PostgreSQL so the output is suitable for Bradley-Terry / Thurstone-style downstream scaling.

## Stack

- NestJS 11 API
- Next.js 16 App Router frontend
- Tailwind CSS 4
- PostgreSQL
- SQL migrations
- VoxCeleb2 CSV import CLI

## What is implemented

- Pairwise rating UI with two recordings and three left/right choices per prompt
- Same-speaker sample replacement when a recording is noisy or unusable
- Phase 1 same-gender pairing
- Phase 2 cross-gender frontier pairing for ambiguous femininity cases
- Prompt selection that balances low coverage with close-score comparisons
- Elo-style online score updates per criterion and phase
- SQL schema with indexes for prompts, votes, scores, recordings, and rejections
- Audio streaming endpoint backed by the source storage directory
- CLI import for VoxCeleb2 metadata

## Environment

Copy `.env.example` to `.env` if you want local overrides. Defaults already match the task:

```bash
PORT=3001
DATABASE_HOST=127.0.0.1
DATABASE_PORT=5432
DATABASE_NAME=libvoice
DATABASE_USER=libvoice
DATABASE_PASSWORD=1111
AUDIO_STORAGE_BASE=/media/data/experiment/voxeleb2
VOXCELEB2_METADATA_PATH=/media/data/experiment/voxeleb2/metadata.csv
VOXCELEB2_SPEAKERS_PATH=/media/data/experiment/voxeleb2/vox2_meta.csv
NEXT_PUBLIC_API_BASE_URL=http://localhost:3001
```

## Commands

Install dependencies:

```bash
npm install
```

Apply migrations:

```bash
npm run db:migrate
```

Import VoxCeleb2 metadata:

```bash
npm run cli -- import-voxceleb2
```

Useful import variants:

```bash
npm run cli -- import-voxceleb2 --dry-run --limit 1000
npm run cli -- import-voxceleb2 --limit 5000 --batch-size 500
```

Run the frontend and backend together in development:

```bash
npm run dev
```

Build both apps:

```bash
npm run build
```

Run the production API only:

```bash
npm run start:api
```

Run the production web client only:

```bash
npm run start:web
```

## API overview

- `GET /api/health`
- `GET /api/comparisons/next?sessionId=<id>&phase=phase1|phase2`
- `POST /api/comparisons/:promptId/vote`
- `POST /api/comparisons/:promptId/reject`
- `GET /api/audio/:recordingId`

## Pairing logic

Phase 1:

- Only compares speakers of the same gender
- Prioritizes speakers with low total coverage
- Prefers opponents with nearby current scores so the service spends more effort on uncertain comparisons

Phase 2:

- Uses phase 1 femininity scores
- Selects from the lowest-scoring female frontier and highest-scoring male frontier
- Prioritizes close cross-gender pairs that are likely to be controversial

## Notes

- Importing only the first few rows of `metadata.csv` usually covers too few speakers for prompt generation. Use a larger slice such as `--limit 5000` for meaningful local verification.
- The frontend build uses `next build --webpack` for reproducible local verification in restricted environments where Turbopack may fail.
