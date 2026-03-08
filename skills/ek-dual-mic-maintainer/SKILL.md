---
name: ek-dual-mic-maintainer
description: Continue development, maintenance, review, handoff, or validation work for the EKDualMic repository. Use when Codex is asked to continue previous work in this repo, understand current project scope, update the durable completion record, or run/adjust Windows-facing tests.
---

# EK Dual Mic Maintainer

## Start Here

Read these files before making implementation decisions:

1. `README.md`
2. `Dual-Mic-Crosstalk-Canceller-README.md`
3. Read only the relevant supporting docs from `docs/`
4. `docs/completed-work.md`
5. If the task includes Windows validation or handoff to a Windows machine, read `docs/windows-test.md`

Use the first two files to understand product scope and constraints. Use `docs/completed-work.md` as the durable status log instead of guessing what was already finished.
If the user reports GUI failures, also inspect the newest file under `logs/` before hypothesizing.

## Repository Rules To Preserve

- Keep the Windows-only product direction unless the user explicitly changes scope.
- Preserve the fixed audio contract: `48 kHz`, `mono`, `10 ms`, `float32`.
- Keep the peer reference path raw on the main chain; do not add AGC, compression, or other dynamic-altering preprocessing unless the project direction explicitly changes.
- Respect the existing crate boundaries before introducing new modules.
- Do not mark WASAPI capture, virtual mic output, or drift compensation as completed until code and validation both exist.

## Completion Record Workflow

After any non-trivial implementation, bug fix, validation pass, or workflow change:

1. Update `docs/completed-work.md` in the same change set.
2. Add an entry with an absolute date, summary of what changed, verification performed, and remaining gaps or risks.
3. If Windows validation steps or expectations changed, update `docs/windows-test.md`.
4. If the main entry points, recommended commands, or handoff docs changed, update `README.md`.

Do not leave project state only in commit diffs or chat history. Write it into the repository docs.

## Validation Defaults

- Start with the smallest relevant validation command.
- If shared runtime or cross-crate behavior changed, run `cargo test --workspace`.
- For mock DSP/runtime changes, run `cargo run -q -p offline_replay -- configs/peer.toml 180` and inspect the generated artifacts.
- For Windows GUI or operator flow changes, use `configs/master.toml` / `configs/slave.toml` / `configs/peer.toml` plus the procedure in `docs/windows-test.md`.
- `configs/peer.toml` is the symmetric preset. Keep `session_mode = "peer"` for `peer/peer`, or change it to `both` for `both/both`.
- For real dual-node LAN work, remember the GUI now exposes `Local Listen Address` and `Peer Address`; the recommended pattern is `0.0.0.0:38001` locally on both machines and the opposite machine's `IP:38001` as `peer_addr`.
- The GUI top bar now includes a language menu for `English` / `中文`; if screenshots or user reports look inconsistent, first confirm which UI language is currently selected.
- Default config presets are now embedded into `app.exe`; when the repo `configs/` directory is absent, GUI preset loading should still work and first save should materialize a local `configs/` directory beside the executable or workspace root.
- The GUI can now import an entire config folder through a native Windows folder picker. Exact-content duplicates should be skipped; same-name different-content files should trigger a warning and, on confirmation, be renamed with incrementing suffixes like `-1`, `-2` rather than `-1-1`.

## Handoff Expectations

When handing off, leave enough repository state that another Codex can continue without chat history:

- `README.md` explains where to start
- `docs/completed-work.md` says what is done and what is still missing
- `docs/windows-test.md` explains how to validate on Windows
- `logs/` should be checked first for any GUI panic or transient runtime failure report

If any of those become stale, fix them before finishing.
