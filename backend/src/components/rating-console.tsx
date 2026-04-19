"use client";

import {
  startTransition,
  useEffect,
  useEffectEvent,
  useRef,
  useState,
} from "react";
import { absoluteApiPath } from "@/lib/api-base";
import {
  ratingCriteria,
  type ComparisonPromptDto,
  type ComparisonSide,
  type NextPromptResponseDto,
  type RatingPhase,
  type RejectRecordingRequestDto,
  type RejectRecordingResponseDto,
  type SubmitVoteRequestDto,
  type SubmitVoteResponseDto,
  type UiCriterionId,
} from "@/shared/contracts";

type ChoiceState = Partial<Record<UiCriterionId, ComparisonSide>>;

const playbackHotkeys: Record<ComparisonSide, string> = {
  left: "1",
  right: "2",
};

const answerHotkeys = [
  { left: "Q", right: "W" },
  { left: "A", right: "S" },
  { left: "Z", right: "X" },
] as const;

const phaseDescriptions: Record<RatingPhase, string> = {
  phase1: "Same-gender coverage and close-speaker comparisons.",
  phase2: "Cross-gender frontier comparisons near the femininity boundary.",
};

function ensureSessionId() {
  const storageKey = "voicelib-session-id";
  const existingId = window.localStorage.getItem(storageKey);

  if (existingId) {
    return existingId;
  }

  const nextId = window.crypto.randomUUID();
  window.localStorage.setItem(storageKey, nextId);
  return nextId;
}

async function parseApiResponse<T>(response: Response) {
  if (!response.ok) {
    let message = `Request failed (${response.status})`;

    try {
      const payload = (await response.json()) as { message?: string };
      message = payload.message ?? message;
    } catch {
      const payload = await response.text();
      message = payload || message;
    }

    throw new Error(message);
  }

  return (await response.json()) as T;
}

async function fetchNextPrompt(sessionId: string, phase: RatingPhase) {
  const url = new URL(absoluteApiPath("/api/comparisons/next"));
  url.searchParams.set("sessionId", sessionId);
  url.searchParams.set("phase", phase);

  const response = await fetch(url, {
    cache: "no-store",
  });
  const payload = await parseApiResponse<NextPromptResponseDto>(response);
  return payload.prompt;
}

async function submitVote(promptId: number, body: SubmitVoteRequestDto) {
  const response = await fetch(absoluteApiPath(`/api/comparisons/${promptId}/vote`), {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
    },
    body: JSON.stringify(body),
  });

  return parseApiResponse<SubmitVoteResponseDto>(response);
}

async function rejectRecording(promptId: number, body: RejectRecordingRequestDto) {
  const response = await fetch(
    absoluteApiPath(`/api/comparisons/${promptId}/reject`),
    {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
      },
      body: JSON.stringify(body),
    },
  );

  return parseApiResponse<RejectRecordingResponseDto>(response);
}

function formatError(error: unknown) {
  return error instanceof Error ? error.message : "Unexpected error.";
}

function isEditableTarget(target: EventTarget | null) {
  if (!(target instanceof HTMLElement)) {
    return false;
  }

  const tagName = target.tagName.toLowerCase();

  return (
    target.isContentEditable ||
    tagName === "input" ||
    tagName === "textarea" ||
    tagName === "select"
  );
}

export default function RatingConsole() {
  const sessionIdRef = useRef("");
  const leftAudioRef = useRef<HTMLAudioElement | null>(null);
  const rightAudioRef = useRef<HTMLAudioElement | null>(null);
  const [phase, setPhase] = useState<RatingPhase>("phase1");
  const [prompt, setPrompt] = useState<ComparisonPromptDto | null>(null);
  const [choices, setChoices] = useState<ChoiceState>({});
  const [isLoading, setIsLoading] = useState(true);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [isRejecting, setIsRejecting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let isActive = true;
    sessionIdRef.current = ensureSessionId();
    setIsLoading(true);
    setError(null);

    void (async () => {
      try {
        const nextPrompt = await fetchNextPrompt(sessionIdRef.current, phase);

        if (!isActive) {
          return;
        }

        startTransition(() => {
          setPrompt(nextPrompt);
          setChoices({});
        });
      } catch (loadError) {
        if (isActive) {
          setError(formatError(loadError));
        }
      } finally {
        if (isActive) {
          setIsLoading(false);
        }
      }
    })();

    return () => {
      isActive = false;
    };
  }, [phase]);

  const completedAllCriteria = ratingCriteria.every(
    (criterion) => choices[criterion.id],
  );

  function selectChoice(criterionId: UiCriterionId, side: ComparisonSide) {
    setChoices((currentChoices) => ({
      ...currentChoices,
      [criterionId]: side,
    }));
  }

  async function toggleRecording(side: ComparisonSide) {
    const activeAudio = side === "left" ? leftAudioRef.current : rightAudioRef.current;
    const otherAudio = side === "left" ? rightAudioRef.current : leftAudioRef.current;

    if (!activeAudio) {
      return;
    }

    if (!activeAudio.paused) {
      activeAudio.pause();
      return;
    }

    otherAudio?.pause();

    try {
      await activeAudio.play();
    } catch (playError) {
      setError(formatError(playError));
    }
  }

  async function handleSubmit() {
    if (!prompt || !completedAllCriteria) {
      return;
    }

    setIsSubmitting(true);
    setError(null);

    try {
      const response = await submitVote(prompt.id, {
        sessionId: sessionIdRef.current,
        choices: choices as Record<UiCriterionId, ComparisonSide>,
      });

      startTransition(() => {
        setPrompt(response.nextPrompt);
        setChoices({});
      });
    } catch (submitError) {
      setError(formatError(submitError));
    } finally {
      setIsSubmitting(false);
    }
  }

  async function handleReject(recordingId: number) {
    if (!prompt) {
      return;
    }

    setIsRejecting(true);
    setError(null);

    try {
      const response = await rejectRecording(prompt.id, {
        sessionId: sessionIdRef.current,
        recordingId,
        reason: "quality",
      });

      startTransition(() => {
        setPrompt(response.prompt);
      });
    } catch (rejectError) {
      setError(formatError(rejectError));
    } finally {
      setIsRejecting(false);
    }
  }

  const handleGlobalKeyDown = useEffectEvent((event: KeyboardEvent) => {
    if (event.repeat || event.metaKey || event.ctrlKey || event.altKey) {
      return;
    }

    if (isEditableTarget(event.target)) {
      return;
    }

    const normalizedKey = event.key.toLowerCase();

    if (normalizedKey === playbackHotkeys.left) {
      event.preventDefault();
      void toggleRecording("left");
      return;
    }

    if (normalizedKey === playbackHotkeys.right) {
      event.preventDefault();
      void toggleRecording("right");
      return;
    }

    if (!prompt || isLoading || isSubmitting || isRejecting) {
      return;
    }

    if (normalizedKey === " ") {
      event.preventDefault();
      void handleSubmit();
      return;
    }

    for (const [criterionIndex, criterion] of ratingCriteria.entries()) {
      const hotkeys = answerHotkeys[criterionIndex];

      if (!hotkeys) {
        continue;
      }

      if (normalizedKey === hotkeys.left.toLowerCase()) {
        event.preventDefault();
        selectChoice(criterion.id, "left");
        return;
      }

      if (normalizedKey === hotkeys.right.toLowerCase()) {
        event.preventDefault();
        selectChoice(criterion.id, "right");
        return;
      }
    }
  });

  useEffect(() => {
    window.addEventListener("keydown", handleGlobalKeyDown);

    return () => {
      window.removeEventListener("keydown", handleGlobalKeyDown);
    };
  }, [handleGlobalKeyDown]);

  return (
    <main className="mx-auto flex w-full max-w-7xl flex-1 flex-col px-5 py-8 sm:px-8 lg:px-12">
      <section className="rounded-[2rem] border border-white/30 bg-white/70 p-6 shadow-[0_20px_80px_rgba(63,52,34,0.12)] backdrop-blur md:p-8">
        <div className="flex flex-col gap-6">
          <div className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
            <div className="max-w-2xl">
              <p className="font-mono text-xs uppercase tracking-[0.32em] text-stone-500">
                VoiceLib rating console
              </p>
              <h1 className="mt-3 text-4xl font-semibold tracking-tight text-stone-900 sm:text-5xl">
                Pairwise voice scoring for Bradley-Terry style training.
              </h1>
              <p className="mt-3 max-w-xl text-sm leading-6 text-stone-600 sm:text-base">
                Rate the pair on all three axes, then move straight to the next
                comparison. Reject a noisy sample to swap in another recording
                from the same speaker.
              </p>
              <p className="mt-3 max-w-2xl text-xs leading-6 text-stone-500 sm:text-sm">
                Hotkeys: <span className="font-mono">1</span>/<span className="font-mono">2</span> play
                or pause left/right, <span className="font-mono">Q W</span>,{" "}
                <span className="font-mono">A S</span>,{" "}
                <span className="font-mono">Z X</span> answer rows 1-3,{" "}
                <span className="font-mono">Space</span> submits.
              </p>
            </div>

            <div className="grid gap-3 sm:grid-cols-2">
              {(["phase1", "phase2"] as const).map((phaseOption) => (
                <button
                  key={phaseOption}
                  type="button"
                  onClick={() => setPhase(phaseOption)}
                  className={`rounded-2xl border px-4 py-3 text-left transition ${
                    phase === phaseOption
                      ? "border-stone-900 bg-stone-900 text-stone-50"
                      : "border-stone-300 bg-stone-50/80 text-stone-700 hover:border-stone-500"
                  }`}
                >
                  <div className="font-mono text-[11px] uppercase tracking-[0.28em]">
                    {phaseOption === "phase1" ? "Phase 1" : "Phase 2"}
                  </div>
                  <div className="mt-2 text-sm leading-5">
                    {phaseDescriptions[phaseOption]}
                  </div>
                </button>
              ))}
            </div>
          </div>

          <div className="rounded-[1.5rem] border border-stone-200/80 bg-stone-950 px-5 py-4 text-stone-100">
            <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
              <div>
                <div className="font-mono text-[11px] uppercase tracking-[0.28em] text-stone-400">
                  Current strategy
                </div>
                <div className="mt-2 text-sm text-stone-100">
                  {prompt?.selectionReason ?? "Loading prompt selection..."}
                </div>
              </div>
              <div>
                <div className="font-mono text-[11px] uppercase tracking-[0.28em] text-stone-400">
                  Your completed prompts
                </div>
                <div className="mt-2 text-2xl font-semibold">
                  {prompt?.sessionProgress.completedPrompts ?? 0}
                </div>
              </div>
              <div>
                <div className="font-mono text-[11px] uppercase tracking-[0.28em] text-stone-400">
                  Your recorded votes
                </div>
                <div className="mt-2 text-2xl font-semibold">
                  {prompt?.sessionProgress.completedVotes ?? 0}
                </div>
              </div>
              <div>
                <div className="font-mono text-[11px] uppercase tracking-[0.28em] text-stone-400">
                  Approx. comparisons left
                </div>
                <div className="mt-2 text-2xl font-semibold">
                  {prompt?.phaseEstimate.estimatedRemainingPrompts ?? "—"}
                </div>
                <div className="mt-1 text-xs text-stone-400">
                  {prompt?.phaseEstimate.progressPercent ?? 0}% of this phase target
                </div>
              </div>
            </div>

            <div className="mt-4 border-t border-white/10 pt-4 text-xs leading-5 text-stone-400">
              {prompt?.phaseEstimate.note ??
                "Estimating remaining comparisons for the current phase..."}
            </div>
          </div>

          {error ? (
            <div className="rounded-2xl border border-rose-300 bg-rose-50 px-4 py-3 text-sm text-rose-900">
              {error}
            </div>
          ) : null}

          {isLoading || !prompt ? (
            <div className="rounded-[2rem] border border-dashed border-stone-300 bg-stone-50 px-6 py-16 text-center text-stone-500">
              Loading the next rating prompt...
            </div>
          ) : (
            <>
              <div className="grid gap-5 lg:grid-cols-2">
                {[prompt.left, prompt.right].map((recording) => (
                  <article
                    key={recording.recordingId}
                    className="relative overflow-hidden rounded-[2rem] border border-stone-200 bg-[linear-gradient(145deg,rgba(255,255,255,0.92),rgba(250,240,221,0.9))] p-5 shadow-[0_18px_50px_rgba(0,0,0,0.08)]"
                  >
                    <div className="absolute inset-x-6 top-0 h-px bg-[linear-gradient(90deg,transparent,rgba(39,35,29,0.35),transparent)]" />
                    <div className="flex items-start justify-between gap-4">
                      <div>
                        <div className="font-mono text-xs uppercase tracking-[0.3em] text-stone-500">
                          {recording.label}
                        </div>
                        <div className="mt-2 text-2xl font-semibold tracking-tight text-stone-900">
                          Listen before scoring
                        </div>
                        <div className="mt-2 inline-flex rounded-full bg-stone-200 px-3 py-1 font-mono text-[11px] uppercase tracking-[0.24em] text-stone-700">
                          {playbackHotkeys[recording.side]}: play / pause
                        </div>
                      </div>
                      <button
                        type="button"
                        onClick={() => handleReject(recording.recordingId)}
                        disabled={isRejecting || isSubmitting}
                        className="rounded-full border border-stone-300 px-3 py-2 text-xs font-medium uppercase tracking-[0.2em] text-stone-600 transition hover:border-stone-500 hover:text-stone-900 disabled:cursor-not-allowed disabled:opacity-50"
                      >
                        Replace sample
                      </button>
                    </div>

                    <div className="mt-6 rounded-[1.5rem] bg-stone-900/95 p-4 text-stone-50">
                      <audio
                        ref={recording.side === "left" ? leftAudioRef : rightAudioRef}
                        controls
                        preload="metadata"
                        className="w-full"
                        src={absoluteApiPath(recording.audioPath)}
                      >
                        Your browser does not support audio playback.
                      </audio>
                    </div>
                  </article>
                ))}
              </div>

              <div className="rounded-[2rem] border border-stone-200 bg-white/90 p-5 shadow-[0_14px_40px_rgba(0,0,0,0.05)]">
                <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
                  <div>
                    <div className="font-mono text-xs uppercase tracking-[0.3em] text-stone-500">
                      Rating panel
                    </div>
                    <h2 className="mt-2 text-2xl font-semibold tracking-tight text-stone-900">
                      Choose the stronger recording on each scale.
                    </h2>
                  </div>
                  <div className="text-sm text-stone-500">
                    {completedAllCriteria
                      ? "All three scores selected."
                      : `${ratingCriteria.filter((criterion) => choices[criterion.id]).length} of ${ratingCriteria.length} answered.`}
                  </div>
                </div>

                <div className="mt-6 grid gap-4">
                  {ratingCriteria.map((criterion, criterionIndex) => (
                    <div
                      key={criterion.id}
                      className="grid gap-3 rounded-[1.4rem] border border-stone-200 bg-stone-50/80 p-4 lg:grid-cols-[1.3fr,1fr,1fr]"
                    >
                      <div>
                        <div className="font-mono text-[11px] uppercase tracking-[0.28em] text-stone-500">
                          {criterion.shortLabel}
                        </div>
                        <div className="mt-2 text-sm leading-6 text-stone-700">
                          {criterion.label}
                        </div>
                        <div className="mt-3 inline-flex rounded-full bg-stone-200 px-3 py-1 font-mono text-[11px] uppercase tracking-[0.24em] text-stone-700">
                          {answerHotkeys[criterionIndex]?.left} / {answerHotkeys[criterionIndex]?.right}
                        </div>
                      </div>

                      {[prompt.left, prompt.right].map((recording) => {
                        const active = choices[criterion.id] === recording.side;

                        return (
                          <button
                            key={`${criterion.id}-${recording.side}`}
                            type="button"
                            onClick={() => selectChoice(criterion.id, recording.side)}
                            disabled={isSubmitting || isRejecting}
                            className={`rounded-[1.3rem] border px-4 py-4 text-left transition ${
                              active
                                ? "border-emerald-700 bg-emerald-700 text-white shadow-[0_10px_24px_rgba(6,95,70,0.25)]"
                                : "border-stone-300 bg-white text-stone-800 hover:border-stone-500"
                            } disabled:cursor-not-allowed disabled:opacity-60`}
                          >
                            <div className="font-mono text-[11px] uppercase tracking-[0.28em] opacity-70">
                              Pick {recording.side === "left"
                                ? answerHotkeys[criterionIndex]?.left
                                : answerHotkeys[criterionIndex]?.right}
                            </div>
                            <div className="mt-2 text-lg font-semibold">
                              {recording.label}
                            </div>
                          </button>
                        );
                      })}
                    </div>
                  ))}
                </div>

                <div className="mt-6 flex flex-col gap-4 border-t border-stone-200 pt-5 sm:flex-row sm:items-center sm:justify-between">
                  <p className="max-w-xl text-sm leading-6 text-stone-600">
                    Replacements keep the speaker fixed, so you can reject noise
                    without losing the comparison pair.
                  </p>
                  <button
                    type="button"
                    onClick={handleSubmit}
                    disabled={!completedAllCriteria || isSubmitting || isRejecting}
                    className="rounded-full bg-stone-950 px-6 py-3 text-sm font-semibold text-stone-50 transition hover:bg-stone-800 disabled:cursor-not-allowed disabled:bg-stone-300"
                  >
                    {isSubmitting ? "Saving and loading next pair..." : "Submit and continue"}
                  </button>
                </div>
              </div>
            </>
          )}
        </div>
      </section>
    </main>
  );
}
