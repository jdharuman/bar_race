# Daily Automation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One command per pipeline step (pick → research → render → upload → finish) that n8n runs daily, plus the research skill, setup skill, rules and n8n workflow in the repo.

**Architecture:** `automation/daily.py` (stdlib only: queue, folders, research and render wrappers, finish) and `automation/youtube.py` (Google API upload). Each step prints one JSON line on stdout and writes `error.txt` in the video folder on failure, so n8n only chains commands and never quotes error text. Research is Claude Code headless (`claude -p`) told to follow `.claude/skills/barrace-research/SKILL.md`.

**Tech Stack:** Python 3.12 via uv, pytest, google-api-python-client, google-auth-oauthlib, Claude Code CLI 2.x, n8n (host install), existing `barrace` Rust binary.

**Spec:** `docs/superpowers/specs/2026-11-21-daily-automation-design.md`

## Global Constraints

- Uploads are `privacyStatus: "private"` only; no code path sets public/unlisted or deletes videos.
- Secrets live in `~/.config/barrace/` (`client_secret.json`, `youtube_token.json`), never in the repo.
- `topics.csv`, `topics_done.csv`, `videos/` are git-ignored; repo ships `topics.example.csv`.
- topics columns: `style,title,youtube_url,comment`; done adds `finished_at,folder,status,main_video_id,short_video_id,error`.
- `style` ∈ {`race`,`pillars`}; at least one of `title`/`youtube_url`.
- Folders: `videos/YYYY-MM-DD-<slug>/`.
- Research: `--model opus`, 45-minute timeout (2700 s), one retry after 30 min on usage-limit errors.
- YouTube: category `27`, `selfDeclaredMadeForKids: false`, Short title/description contain `#Shorts`.
- Studio link format: `https://studio.youtube.com/video/<id>/edit`.
- Python runs through `uv` (system Python is 3.8).

## Review Focus

1. Malformed first row in `topics.csv` (bad style, no title/url) → row moves to done as `failed` with the reason, run reports it; queue doesn't jam forever. (Task 1 test)
2. Two runs overlap (manual + scheduled) → second exits 4 without touching the queue; a stale lock (>6 h) is ignored. (Task 1 test)
3. User edited `topics.csv` while a run was in flight (added rows at top) → `finish` removes the exact row it picked, not "row 1". (Task 2 test)
4. Upload fails after the main video → retry skips main, uploads only the Short. (Task 4 test)
5. Claude run succeeds but leaves invalid/missing files → research step fails with the validate output in `error.txt`. (Task 3 test)

---

### Task 1: Queue and pick

**Files:**
- Create: `automation/pyproject.toml` (uv project, python ≥3.12, deps: google-api-python-client, google-auth-oauthlib; dev: pytest)
- Create: `automation/daily.py`
- Create: `automation/tests/test_daily.py`
- Modify: `.gitignore` (add `/topics.csv`, `/topics_done.csv`, `/videos/`)
- Create: `topics.example.csv`

**Interfaces:**
- Produces:
  - `TOPIC_COLS = ["style","title","youtube_url","comment"]`, `DONE_COLS = TOPIC_COLS + ["finished_at","folder","status","main_video_id","short_video_id","error"]`
  - `read_rows(path: Path) -> list[dict]` (missing file → `[]`), `write_rows(path, cols, rows)`
  - `row_problem(row: dict) -> str | None`
  - `slugify(row: dict) -> str` — title → lowercase, non-alnum runs → `-`, trimmed, ≤50 chars; no title → `yt-<video id>`
  - `video_id(url: str) -> str | None` (watch?v=, youtu.be/, shorts/)
  - `find_previous(row, done_rows, root) -> Path | None` — latest done row, same style, same normalised title (lowercase, collapsed spaces) or same `video_id`, whose folder exists
  - `pick(root: Path, today: date) -> dict` — returns `{"dir": str, "style", "title", "youtube_url", "redo_of": str|None}`; writes `<dir>/topic.json` = `{"row": row, "redo_of": ..., "picked_at": iso}`; creates `videos/.lock`
  - CLI: `daily.py [--root R] pick` prints the JSON; exit 3 `{"empty": true}`; exit 4 `{"locked": true}`; exit 2 on invalid row (after moving it to done as failed)

- [ ] **Step 1: Tests** in `test_daily.py` using `tmp_path` as root:
  - `test_pick_creates_folder_and_topic_json`: topics row `race,Largest Economies,,` → dir `videos/2026-11-22-largest-economies`, `topic.json["row"]["title"] == "Largest Economies"`, topics.csv unchanged.
  - `test_pick_same_day_twice_suffixes`: existing folder → `...-largest-economies-2`.
  - `test_slug_from_url`: `slugify({"title":"","youtube_url":"https://youtu.be/abc123"}) == "yt-abc123"`.
  - `test_pick_empty_exit3` (CLI via subprocess, `sys.executable daily.py --root tmp pick`).
  - `test_pick_locked_exit4` and `test_stale_lock_ignored` (lock mtime set 7 h back via `os.utime`).
  - `test_invalid_row_moves_to_done`: row `bars,X,,` → exit 2, topics.csv empty, done row status `failed`, error mentions `style`.
  - `test_redo_copies_previous`: done row with folder containing `data.csv`, `out/main.mp4`, `upload.json`; topics row same title + comment → new folder has `data.csv`, no `out/`, no `upload.json`; `redo_of` set.
- [ ] **Step 2:** `cd automation && uv run pytest -q` → fails (no module).
- [ ] **Step 3:** Implement. Lock file `videos/.lock` (stale after 6 h). Copy for redo with `shutil.copytree(ignore=ignore_patterns("out","upload.json","error.txt"))`.
- [ ] **Step 4:** Tests pass.
- [ ] **Step 5:** Commit `feat(automation): topic queue and pick`.

### Task 2: Finish

**Files:** Modify `automation/daily.py`, `automation/tests/test_daily.py`

**Interfaces:**
- Produces: `finish(root, dir: Path, status: str, now: datetime) -> dict` → `{"status","title","main_url","short_url","error","message"}`; CLI `daily.py finish <dir> --status uploaded|failed` (error text read from `<dir>/error.txt`, ids from `<dir>/upload.json`). Removes `videos/.lock`. `message` is the ready-to-send Telegram text.

- [ ] **Step 1: Tests:**
  - `test_finish_moves_exact_row`: topics has rows A,B; pick A; then user prepends row C → finish removes A only; topics = C,B; done has A with `status=uploaded`, `folder`, ids from upload.json `{"main":"m1","short":"s1"}`.
  - `test_finish_failed_records_error_and_partial_ids`: `error.txt` = "research: boom", upload.json `{"main":"m1"}` → done row `failed`, `error` contains `boom`, `main_video_id=m1`; message contains `FAILED` and `boom`.
  - `test_finish_message_links`: message contains `https://studio.youtube.com/video/m1/edit`.
  - `test_finish_releases_lock`.
- [ ] **Step 2:** Run → fail. **Step 3:** Implement (row match: first row equal on all `TOPIC_COLS`). **Step 4:** pass. **Step 5:** Commit `feat(automation): finish step`.

### Task 3: Research and render wrappers

**Files:** Modify `automation/daily.py`, tests.

**Interfaces:**
- Produces:
  - `claude_cmd(dir: Path, root: Path) -> list[str]` = `["claude","-p",<prompt>,"--model","opus","--permission-mode","acceptEdits","--allowedTools",<tools>,"--output-format","json"]`; prompt names the folder and says: read `.claude/skills/barrace-research/SKILL.md` and follow it for `<dir>/topic.json`. Tools: `Read Write Edit Glob Grep WebSearch WebFetch Bash(curl:*) Bash(uv run:*) Bash(yt-dlp:*) Bash(<barrace> validate:*) Bash(ls:*) Bash(mkdir:*) Bash(unzip:*) Bash(file:*)`.
  - `barrace_bin(root) -> str` — env `BARRACE` else `<root>/target/release/barrace`.
  - `research(root, dir, run=subprocess.run, sleep=time.sleep) -> None` — runs claude (cwd root, timeout 2700); on output containing `usage limit` or `limit reached` (case-insensitive) sleeps 1800 and retries once; then requires `metadata.json` and (`data.csv` or `pillars.csv`), then `barrace validate <dir>` must exit 0. Any failure raises `StepError(msg)`.
  - `render(root, dir, run=subprocess.run)` — `barrace render <dir> --format both`, then `barrace thumb <dir>`.
  - CLI `daily.py research <dir>` / `daily.py render <dir>`: on `StepError` or timeout write `<dir>/error.txt` = `"<step>: <msg>\n<last 30 lines of output>"`, exit 1; success prints `{"ok": true}`.
- [ ] **Step 1: Tests** with a fake `run` recording calls:
  - `test_claude_cmd_uses_opus_and_skill`: `"opus"` in cmd; prompt contains `barrace-research/SKILL.md` and the dir.
  - `test_research_retries_on_usage_limit`: first fake result stdout `"Claude usage limit reached"`, second ok → `sleep` called with 1800, claude ran twice.
  - `test_research_fails_without_metadata`: fake claude ok, no files → `StepError` mentioning `metadata.json`.
  - `test_research_fails_on_validate_error`: files present, validate returncode 1 stdout `"error: row 3"` → StepError contains `row 3`.
  - `test_cli_writes_error_txt` (render with `BARRACE=/bin/false`) → exit 1, `error.txt` starts with `render:`.
- [ ] **Steps 2–5:** fail, implement, pass, commit `feat(automation): research and render steps`.

### Task 4: YouTube upload

**Files:** Create `automation/youtube.py`, `automation/tests/test_youtube.py`; modify `daily.py` (subcommands `upload <dir>`, `auth`).

**Interfaces:**
- Consumes: `metadata.json` = `{"main": {"title","description","tags":[...]}, "short": {...}}`; `out/main.mp4`, `out/short.mp4`, `out/thumb.png`.
- Produces:
  - `build_body(meta: dict, short: bool) -> dict` — title ≤100 chars, description ≤5000, tags trimmed to ≤500 total chars, `categoryId "27"`, `status {"privacyStatus":"private","selfDeclaredMadeForKids":False}`; short → ensure `#Shorts` in title (if it fits) and description.
  - `upload_all(dir: Path, yt) -> dict` where `yt` has `insert(path: Path, body: dict) -> str` and `set_thumbnail(video_id: str, path: Path)`. Writes `upload.json` after each video (`{"main": id, "short": id, "thumbnail": true}`); skips keys already present.
  - `class YouTube` (real client: `MediaFileUpload(resumable=True)` loop on `next_chunk`), `load_credentials()` from `~/.config/barrace/youtube_token.json` refreshing; `RefreshError` → `StepError("YouTube login expired: run `uv run automation/daily.py auth`")`.
  - `auth()` — `InstalledAppFlow.from_client_secrets_file(~/.config/barrace/client_secret.json, ["https://www.googleapis.com/auth/youtube.upload"]).run_local_server(port=8080, open_browser=False)`, saves token (mode 0600).
- [ ] **Step 1: Tests:**
  - `test_body_private_and_category`; `test_short_body_has_shorts_tag`; `test_long_title_truncated` (150 chars → 100); `test_tags_capped_500`.
  - `test_upload_resumes`: upload.json `{"main":"m1"}` + fake yt → only short inserted, thumbnail set on `m1`, final json has both.
  - `test_upload_never_public`: every body passed to fake insert has `privacyStatus == "private"`.
- [ ] **Steps 2–5:** fail, implement, pass, commit `feat(automation): youtube upload`.

### Task 5: Rules, research skill, setup skill, n8n workflow, docs

**Files:**
- Create: `CLAUDE.md` — hard rules (never publish/unlist/delete videos; never commit secrets or `topics*.csv`/`videos/`; free-licence images only; estimated values flagged; no reuse of other creators' video/art/music), pointers to both skills and `docs/automation.md`.
- Create: `.claude/skills/barrace-research/SKILL.md` — input `topic.json`; steps: (1) if `youtube_url`, `yt-dlp --dump-json --skip-download` for title/description/duration only; (2) pick sources (World Bank API, OWID CSV, Wikipedia tables, official stats); download whole series with curl/uv-run Python; (3) data through latest available period; (4) write `data.csv`/`pillars.csv` exactly per README formats, `estimated=true` for gap fills; (5) images: flags from public-domain source, people/logos only Wikimedia Commons free-licence (check licence via Commons API `extmetadata`), else none; (6) `video.toml` with `reviewed = true`, style, units, `number_style`; optional `events.csv` (race, 3–8 captions); (7) `metadata.json` (main + short; description lists sources; 10–15 tags); (8) `research_notes.md` (sources, estimates, image licences); (9) run `barrace validate <dir>` until no errors; redo rows: read `comment`, keep existing files and fix only what it names.
- Create: `.claude/skills/barrace-setup/SKILL.md` — ordered steps for Claude on a fresh machine: apt (ffmpeg, build-essential, git, curl, fonts), rustup + `cargo build --release`, uv + `uv tool install --python 3.12 "yt-dlp[default]"`, node 22 + `npm i -g n8n`, Claude Code + `claude setup-token` (user runs, stores `CLAUDE_CODE_OAUTH_TOKEN` in `~/.config/barrace/env`), Google Cloud project + YouTube Data API + OAuth desktop client + consent screen **In production** + apply for audit (user steps, with links), `daily.py auth` via `ssh -L 8080:localhost:8080`, Telegram bot token/chat id, systemd user service for n8n (`N8N_RUNNERS…`/`NODES_EXCLUDE=[]` to enable Execute Command, `EnvironmentFile=~/.config/barrace/env`, `loginctl enable-linger`), `n8n import:workflow --input=n8n/daily-video.json`, set REPO path, activate, dry run (`pick`/`research`/`render` with a test topic, then one upload).
- Create: `n8n/daily-video.json` — Schedule (02:00) → Execute Command `uv run --project automation automation/daily.py pick` → IF exit code: 3 → Telegram "topic list empty"; 4 → stop; 2 → Telegram from stdout; 0 → research → render → upload → finish `--status uploaded` → Telegram `message`; any step error output → finish `--status failed` → Telegram. Commands `cd $BARRACE_REPO && …`. Telegram node uses credential named `barrace telegram`, chat id from env `TELEGRAM_CHAT_ID`.
- Create: `docs/automation.md` — human runbook: daily flow, topics.csv format, how to redo, how to re-auth, where logs are, quota, audit.
- Modify: `README.md` — "Daily automation" section linking docs; replace "the tool never downloads images" with the free-licence rule.
- [ ] **Step 1:** Write files. **Step 2:** `python3 -c "import json;json.load(open('n8n/daily-video.json'))"` passes; `npx -y n8n@latest import:workflow --input=n8n/daily-video.json` into a throwaway `N8N_USER_FOLDER` succeeds. **Step 3:** Commit `docs(automation): rules, skills, n8n workflow, runbook`.

### Task 6: Dry run on this machine

- [ ] **Step 1:** `cargo build --release`; write a one-row `topics.csv` (`race,Largest Economies by GDP,,`).
- [ ] **Step 2:** `uv run --project automation automation/daily.py pick` → JSON with dir.
- [ ] **Step 3:** `… research <dir>` → exit 0; inspect `data.csv`, `metadata.json`, `research_notes.md`.
- [ ] **Step 4:** `… render <dir>` → `out/main.mp4`, `out/short.mp4`, `out/thumb.png` exist.
- [ ] **Step 5:** `… finish <dir> --status failed` with `error.txt`=`upload: not configured on dev machine` → row in `topics_done.csv`, lock gone. (Upload is verified on the target machine during setup.)
- [ ] **Step 6:** Fix anything the run exposed in the skill; commit; final review; PR to `jdharuman/bar_race`.
