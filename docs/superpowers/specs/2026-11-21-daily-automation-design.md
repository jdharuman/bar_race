# barrace — daily automation (topic list → private YouTube upload)

## Intent

The user wants one finished video pair (main + Short) per day with no manual
work except two things: keeping the topic list and watching the result before
publishing. Today every step after picking the topic is manual (research,
writing CSVs, render, upload). This round automates all of it, driven by n8n
on a Linux machine.

The user will clone this repo onto a **different Linux machine** and tell
Claude Code to set everything up. So the repo itself must carry the full
setup steps, the research rules and the n8n workflow, written for Claude to
follow without this conversation.

## The daily flow (what the user experiences)

1. The user keeps `topics.csv` (365 rows, edited by hand on the machine).
2. Every day at a fixed time, n8n runs the pipeline on the **first row** of
   `topics.csv`.
3. Claude Code (headless, the user's Claude subscription) researches the
   topic, writes the input files, and validates them.
4. barrace renders `main.mp4`, `short.mp4`, `thumb.png`.
5. Both videos upload to YouTube as **private**, with title, description,
   tags and thumbnail.
6. The row moves from `topics.csv` to `topics_done.csv`, with the folder and
   video links filled in.
7. The user gets a message with both YouTube Studio links (or the error).
8. The user watches. Good → publishes by hand in YouTube Studio.
   Wrong → inserts the same topic again at the **top** of `topics.csv` with a
   `comment` describing the mistakes. The next run redoes it.

## Goals

1. **Topic list.** `topics.csv` in the repo root, columns:
   ```csv
   style,title,youtube_url,comment
   race,Largest Economies by GDP,,
   pillars,Highest Paid NBA Player by Season,https://www.youtube.com/watch?v=q9EjG56OjN4,
   race,,https://www.youtube.com/watch?v=gyHSFZ4fIh4,
   ```
   - `style`: `race` or `pillars` (required).
   - `title` and/or `youtube_url`: at least one.
   - `comment`: optional; non-empty means "redo with these fixes".
   - The picker takes the first data row. File order is the queue.
   - `topics_done.csv` = same columns + `finished_at, folder, status
     (uploaded|failed), main_video_id, short_video_id, error`.
2. **Research by Claude Code headless.** The pipeline runs
   `claude -p` inside the new video folder with a repo skill
   (`.claude/skills/barrace-research/SKILL.md`) that holds all rules. Output:
   - `data.csv` (race) or `pillars.csv` (pillars), plus `images/`.
   - `video.toml` with `reviewed = true` (the human review moves to the
     finished video; see Decisions).
   - optional `events.csv` (race) with fact captions.
   - `metadata.json`: YouTube title, description (with sources), tags, for
     the main video and the Short.
   - `research_notes.md`: sources used, what was estimated, image licences.
   - The agent runs `barrace validate` and fixes until it passes with no
     errors.
3. **Data runs to the current date.** Series end at the latest available
   period (current year or latest published year), not where the reference
   video stopped.
4. **YouTube link = topic hint only.** The agent reads title, description
   and duration via `yt-dlp` metadata to learn the topic, the item set and
   time range. Numbers come from real sources (World Bank, OWID, Wikipedia,
   official stats). It never downloads or reuses the other creator's video,
   images or music.
5. **Redo.** A row with a `comment`: the picker finds the latest
   `topics_done.csv` entry with the same `style` + `title`/`youtube_url`,
   copies that folder to a new dated folder, and the agent fixes what the
   comment says (keeping the rest). If no earlier folder exists, it
   researches from scratch with the comment as extra instructions.
6. **Upload.** Python (google-api-python-client), OAuth token stored outside
   the repo. Uploads `main.mp4` and `short.mp4` as `private`, category
   Education (27), `madeForKids=false`, sets the thumbnail on the main video,
   adds `#Shorts` to the Short's title/description. Never sets anything
   public, never deletes videos.
7. **Notification.** n8n sends one Telegram message per run: success with
   both Studio links, or failure with the step name and last error lines.
8. **Portable setup.** A setup skill
   (`.claude/skills/barrace-setup/SKILL.md`) that Claude follows on a fresh
   Linux machine: install deps, build barrace, authenticate Claude Code and
   YouTube, install n8n, import the workflow, enable the daily trigger, run
   one dry run. Plus `docs/automation.md` as the human-readable runbook.
9. **Rules file.** Repo-root `CLAUDE.md` with the hard rules for any Claude
   session in this repo (see Anti-goals), pointing at the two skills.

## Pipeline commands (Approach A)

One Python CLI, `automation/daily.py`, run with `uv run`, subcommands that
n8n calls in order. Each prints one JSON line with its result, so n8n can
pass values on.

| Step | Command | Does |
|---|---|---|
| 1 | `daily.py pick` | Reads row 1 of `topics.csv`, creates `videos/YYYY-MM-DD-<slug>/` (copies previous folder on redo), writes `topic.json` in it. Exit 3 if the list is empty. |
| 2 | `daily.py research <dir>` | Runs `claude -p` with the research skill, an allowlisted tool set, and a time limit. Then runs `barrace validate`; fails if errors remain. |
| 3 | `barrace render <dir> --format both` then `barrace thumb <dir>` | Existing. |
| 4 | `daily.py upload <dir>` | Uploads both videos + thumbnail, writes `upload.json` (video ids). |
| 5 | `daily.py finish <dir> --status uploaded\|failed [--error …]` | Removes the row from `topics.csv`, appends it to `topics_done.csv`. |

n8n workflow (`n8n/daily-video.json`): Schedule trigger → Execute Command
(pick) → (research) → (render) → (upload) → (finish) → Telegram. Any failed
step → `finish --status failed` → Telegram error.

## Non-goals

- Publishing or scheduling videos (the user publishes by hand).
- Deleting the old private video after a redo (the user does it in Studio).
- A web UI or Google Sheet for the topic list (CSV on the machine).
- Picking topics (stays manual).
- Voiceover, translated titles.
- Auto-committing video folders to git.

## Anti-goals (failure even if it "works")

- Anything goes public without the user.
- Reusing another creator's video, frames, artwork or music.
- Images without a free licence (use Wikimedia Commons free-licence files or
  public-domain flags; record the licence in `research_notes.md`).
- Made-up numbers presented as real: every gap-filled value is
  `estimated=true` and listed in `research_notes.md`.
- Secrets (tokens, OAuth files) in the repo.
- A failed run silently skipping a topic: failures stay visible in
  `topics_done.csv` and the Telegram message.

## Constraints

- The user's Claude **subscription**, not an API key: Claude Code headless,
  authenticated on the new machine with `claude setup-token`
  (`CLAUDE_CODE_OAUTH_TOKEN`). Research shares the subscription's usage
  limits.
- YouTube API quota: 10,000 units/day; one run uses ~3,250 (2 uploads at
  1,600 + thumbnail 50).
- Unverified YouTube API projects lock uploads to private, and they can't be
  made public until Google's audit passes. The runbook tells the user to
  apply for the audit at setup time. Custom thumbnails need a phone-verified
  channel.
- System Python on the current machine is 3.8; all Python runs through
  `uv` with Python 3.12.
- The machine is headless Linux; OAuth for YouTube uses a console/copy-URL
  flow.

## Approaches

| | Approach | Trade-off |
|---|---|---|
| **A (picked)** | Steps live in the repo as small commands; n8n only schedules, chains, retries and notifies. | Each step runs and tests without n8n; versioned in git; moves with the repo. |
| B | n8n built-in nodes do most work (YouTube node, Code nodes). | Logic hidden in workflow JSON, hard to test/diff; YouTube node lacks thumbnail/Short options. |
| C | Plain cron + one script, no n8n. | Simplest, but the user wants n8n (run history, retries, dashboard). |

The user chose A implicitly by asking for "a proper plan, skill, rule and
steps documented in the same repo" for Claude to set up on another machine.

## Decisions made by Claude (user may override)

- **Review moves to the video.** The agent sets `reviewed = true` after
  `barrace validate` passes; the old CSV review gate (Option B) is replaced
  by watching the private video, which is what the user described.
- **"Move processed file"** = move the row from `topics.csv` to
  `topics_done.csv`. The queue is file order, so inserting at the top makes
  a redo run next.
- **Folders:** `videos/YYYY-MM-DD-<slug>/` (daily now, so not `weeks/`).
  `videos/*/out/` is git-ignored.
- **n8n runs on the host** (npm global install + systemd user service), not
  in Docker, because its Execute Command node must call `claude`, `barrace`,
  `ffmpeg` and `uv` on the host. Execute Command must be enabled
  (n8n 2.x disables it by default).
- **Time:** 02:00 local daily, so videos are ready in the morning.
- **Failure handling:** a failed run moves the row to `topics_done.csv` with
  `status=failed` and the error; the next day takes the next row. To retry,
  the user re-inserts it at the top (same as a redo). One automatic retry of
  the research step after 30 minutes for usage-limit errors.
- **Notification:** Telegram bot (token + chat id in n8n credentials); free
  and phone-friendly. Email can replace it later.
- **Research time limit:** 45 minutes per run.
- **Concurrency:** one run at a time (lock file); a run that finds the lock
  exits without touching the list.
- **Metadata language:** English.
- **Git:** `topics.csv`, `topics_done.csv` and `videos/` are git-ignored
  (machine-local state; `git pull` for tool updates never touches them). The
  repo ships `topics.example.csv`; setup copies it to `topics.csv`.
- **Partial upload:** `upload` resumes from `upload.json`: a video already
  uploaded is skipped, so a retry never makes duplicates. `finish --status
  failed` still records any ids that did upload.
- **YouTube OAuth app** is set to "In production" (unverified) during
  setup, because "Testing" refresh tokens expire after 7 days. The user
  clicks through Google's "unverified app" warning once. An expired or
  revoked token makes `upload` fail with a clear "re-auth: run …" message
  that lands in Telegram.
- **Photos (user decided):** the agent may download photos of people only
  when they are free-licence files on Wikimedia Commons, recording file URL,
  author and licence in `research_notes.md`. No free photo → no image
  (initials tile). Flags: public-domain sources. The README's "the tool never
  downloads images" line changes to describe this rule.
- **Model (user is on Max):** research runs with Opus, 45-minute limit.
- **`weeks/`** stays as is; the manual flow stays documented in the README.
  Redo matching looks only in `topics_done.csv` / `videos/`.

## Left to the builder

The exact research prompt wording and allowlisted tools, the slug rule, the
flag image source (must be public domain / free licence), JSON shapes of
`topic.json`/`metadata.json`/`upload.json`, how the redo match is done
(normalised title or URL), n8n node layout, and test design for the Python
commands (pick/finish/redo matching are pure functions; upload and research
are thin wrappers checked by hand in the dry run).
