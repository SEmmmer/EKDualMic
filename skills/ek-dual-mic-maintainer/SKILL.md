---
name: ek-dual-mic-maintainer
description: Continue development, maintenance, review, handoff, or validation work for the EKDualMic repository at /home/emmmer/git/EKDualMic. Use when Codex is asked to continue previous work in this repo, understand current project scope, update the durable completion record, or run/adjust Windows-facing tests.
---

# EK Dual Mic Maintainer

## Start Here

Read these files before making implementation decisions:

1. `/home/emmmer/git/EKDualMic/README.md`
2. `/home/emmmer/git/EKDualMic/Dual-Mic-Crosstalk-Canceller-README.md`
3. Read only the relevant supporting docs from `/home/emmmer/git/EKDualMic/docs/`
4. `/home/emmmer/git/EKDualMic/docs/completed-work.md`
5. If the task includes Windows validation or handoff to a Windows machine, read `/home/emmmer/git/EKDualMic/docs/windows-test.md`

Use the first two files to understand product scope and constraints. Use `docs/completed-work.md` as the durable status log instead of guessing what was already finished.

## Repository Rules To Preserve

- Keep the Windows-only product direction unless the user explicitly changes scope.
- Preserve the fixed audio contract: `48 kHz`, `mono`, `10 ms`, `float32`.
- Keep the peer reference path raw on the main chain; do not add AGC, compression, or other dynamic-altering preprocessing unless the project direction explicitly changes.
- Respect the existing crate boundaries before introducing new modules.
- Do not mark WASAPI capture, virtual mic output, or drift compensation as completed until code and validation both exist.

## Completion Record Workflow

After any non-trivial implementation, bug fix, validation pass, or workflow change:

1. Update `/home/emmmer/git/EKDualMic/docs/completed-work.md` in the same change set.
2. Add an entry with an absolute date, summary of what changed, verification performed, and remaining gaps or risks.
3. If Windows validation steps or expectations changed, update `/home/emmmer/git/EKDualMic/docs/windows-test.md`.
4. If the main entry points, recommended commands, or handoff docs changed, update `/home/emmmer/git/EKDualMic/README.md`.

Do not leave project state only in commit diffs or chat history. Write it into the repository docs.

## Validation Defaults

- Start with the smallest relevant validation command.
- If shared runtime or cross-crate behavior changed, run `cargo test --workspace`.
- For mock DSP/runtime changes, run `cargo run -q -p offline_replay -- configs/node-a.toml 180` and inspect the generated artifacts.
- For Windows GUI or operator flow changes, use `/home/emmmer/git/EKDualMic/configs/node-a-mock.toml` plus the procedure in `/home/emmmer/git/EKDualMic/docs/windows-test.md`.

## Handoff Expectations

When handing off, leave enough repository state that another Codex can continue without chat history:

- `README.md` explains where to start
- `docs/completed-work.md` says what is done and what is still missing
- `docs/windows-test.md` explains how to validate on Windows

If any of those become stale, fix them before finishing.
