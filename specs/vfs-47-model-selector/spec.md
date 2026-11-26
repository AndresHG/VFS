# VFS-47 — Model selector beside the question

- **Status**: approved (owner chose the models, D2)
- **Size**: M (one subsystem: the ask dialog and its setting; no schema, no user file, same on
  both platforms). It supersedes VFS-28 R1's «Respuestas más baratas» switch in Ajustes.
- **Links**: issue #220, VFS-28 (ask your archive), VFS-46 R7 and D4 (cheap tier by default)

## Problem

In 0.4.0 the answer model is chosen by a switch in Ajustes, «Respuestas más baratas», on by
default (VFS-46 R7): Luna with an OpenAI key, Haiku with an Anthropic key, and Sol or Sonnet
when it is off. The owner wants the choice where the question is asked, as a model selector
like the Claude app's, naming the models, with the cheap one as the default (owner,
2026-11-26).

## What I know

- The setting is `AiSettings.cheap` in `apps/desktop/src-tauri/src/ai.rs`, saved with the
  provider by `ai_set_settings`, and turned into `AskOptions.cheap` for every question and
  follow-up. `vfs_ai::cost::models_for` names each provider's cheap and mid models.
- The ask dialog is `#ai-ask-form` in `apps/desktop/ui/index.html`, driven by `ask.js`; the
  switch is `#ai-cheap` in the Ajustes AI section, driven by `ai.js`.
- One key is active at a time (`AiStatus.key` for `settings.provider`), so the selector lists
  the active provider's models only.
- Nothing like a selector ships today (grep of `apps/desktop/ui/*.js`).

## Requirements (EARS)

- R1. THE ask dialog SHALL show a model selector beside «Preguntar» listing the active
  provider's two answer models by name, cheap first: «Luna» and «Sol» with OpenAI, «Haiku» and
  «Sonnet» with Anthropic, each with a short cost hint (the cheap one «más barato», the other
  «unas 20 veces más caro»). Supersedes VFS-28 R1's switch.
- R2. THE selector SHALL show the saved choice, the cheap model when nothing is saved
  (VFS-46 R7), and WHEN the user picks a model, THE app SHALL save it at once, so it holds for
  the next question, follow-ups included, and the next launch.
- R3. THE Ajustes AI section SHALL no longer show «Respuestas más baratas»; the saved
  setting keeps its meaning, so an existing choice carries over without migration.
- R4. WHILE a question is being answered, THE selector SHALL be disabled; the record (VFS-28
  R11) keeps naming the model each call used.
- R5. WHEN the provider changes in Ajustes, THE selector SHALL list the new provider's models
  with the same tier selected.

## Acceptance scenarios

Tests cite these ids (`VFS-47 AC-<m>`) in their doc comments.

- AC-1 (R1, R5) Given an OpenAI key, When the ask dialog opens, Then the selector lists Luna
  then Sol; Given an Anthropic key, Then Haiku then Sonnet; both catalogs carry the labels.
- AC-2 (R2) Given no saved settings, Then the cheap model is selected; When the user picks the
  mid model, Then `ai_set_settings` saves `cheap: false` and the next question's request names
  the mid model.
- AC-3 (R3) The Ajustes AI section has no «Respuestas más baratas» control.
- AC-4 (R4) While an answer streams, the selector is disabled, and enabled again after.

## Non-goals

- Top-tier models (Opus) in the selector; the reference run keeps using Opus in tests only.
- Two keys at once, or choosing the rewrite model: the cheap model always rewrites (VFS-28 D7).
- Showing a price per question before asking; the cost line after the answer stays.

## Pre-mortem

- Slow PC / HDD, long paths, NFC/NFD, antivirus, kill or power cut, million-match queries,
  permission-denied folders, user files: n/a because the change is one control and one saved
  flag that already exists; no file or index is read or written beyond `ai-settings.json`.
- Does it already ship? Grepped `apps/desktop/ui/*.js`: only the Ajustes switch.

## Decisions

| Id | Decision | Choice | Who | Date |
|----|----------|--------|-----|------|
| D1 | Where the choice lives | In the ask dialog, replacing the Ajustes switch | owner | 2026-11-26 |
| D2 | Models offered | The provider's cheap and mid models, not Opus | owner | 2026-11-26 |
| D3 | Storage | The existing `cheap` flag, so no migration (R3) | agent | 2026-11-26 |
