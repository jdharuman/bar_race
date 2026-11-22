# Daily automation — runbook

Every day at 02:00, n8n runs `automation/daily.py run`. It makes the video for
the **first row** of `topics.csv` and uploads it to YouTube as **private**,
then sends a Telegram message with the links (or the error). You watch, then
publish by hand in YouTube Studio.

```
topics.csv row 1 ─ pick ─ research (Claude) ─ render (barrace) ─ upload (private) ─ finish ─ Telegram
                    │                                                               │
                    videos/YYYY-MM-DD-<slug>/                     row moves to topics_done.csv
```

Setting it up on a new machine: open Claude Code in the repo and say
"set up barrace automation". It follows `.claude/skills/barrace-setup/SKILL.md`.

## The topic list

`topics.csv` (repo root, not in git; setup copies the 365-day list
`topics.plan.csv` into it, `topics.example.csv` shows the format):

```csv
style,title,youtube_url,comment
race,Largest Economies by GDP,,
pillars,Highest Paid NBA Player by Season,https://www.youtube.com/watch?v=q9EjG56OjN4,
race,,https://www.youtube.com/watch?v=gyHSFZ4fIh4,
```

| column | meaning |
|---|---|
| `style` | `race` (bar chart race), `pillars` (3D pillar walk) `stocks` (line chart: $1,000 invested in 2–3 companies; title = the companies, e.g. `Apple vs Microsoft vs Nvidia`) or `compare` (line chart of 2–3 things over time, e.g. `Messi vs Ronaldo career goals`) |
| `title` | the topic in your words |
| `youtube_url` | optional reference video; used only to learn the topic, never copied |
| `comment` | empty for a new topic; for a redo, what was wrong |
| `look` | optional; `neon` = black background with glowing colours (line charts: stocks, compare), empty = light. Older files without this column still work. |
| `theme` | optional weekly theme: `sports` (Mon), `india` (Tue), `economy` (Wed), `earth` (Thu), `tech` (Fri), `history` (Sat), `people` (Sun). Also the playlist the videos go in. |

The run always takes the **top row**. Reorder rows to change what comes next.
Finished rows move to `topics_done.csv` with the folder, status, video ids and
any error.

## Reviewing and fixing a video

1. The Telegram message has Studio links for the main video and the Short.
2. Good: in Studio, set visibility to Public (or schedule it).
3. Wrong: add the same topic again **at the top** of `topics.csv` with a
   comment, e.g.
   `race,Largest Economies by GDP,,"India missing before 1990; Japan 1995 value too high"`.
   The next run copies the previous folder, fixes what the comment says, and
   uploads a new private pair. Delete the old private videos in Studio.
4. A failed run (❌ message): fix the cause if it's setup (see below), then put
   the row back at the top **without a comment**. The retry continues in the
   same folder and skips the steps that already worked, so a half-finished
   upload never uploads twice. If you don't, the next day's run moves on to
   the next row.

Want it now instead of 02:00? In n8n, open "barrace daily video" → "Execute
workflow", or on the machine:
`cd ~/barrace && uv run --project automation automation/daily.py run`.

## Nightly slots, weekly themes and the YouTube quota

Each night makes up to **three video pairs**, one per slot, in this order:
**race**, **pillars**, **lines** (`stocks` or `compare`). Each slot takes the
first topic of its kind whose `theme` is today's theme, else the first one
without a theme, else the first one of that kind. A slot with no topic is
skipped (one Telegram note). The success message names the playlist
(`Playlist: Sports`); add the videos to it in Studio.

YouTube gives the API project **10,000 units a day** (reset at midnight
Pacific, 09:00 in Berlin). A pair costs about 3,250 (1,600 per video + 50 for
the thumbnail), so three pairs fit. The run keeps a ledger in
`videos/.quota.json` and never starts an upload the day can't cover: the
video is made, the message says "⏳ upload waiting", and it uploads first
thing in the next run. Telegram requests use the same day's quota. When
Google raises the quota, set `BARRACE_QUOTA=<new limit>` in
`~/.config/barrace/env` and `systemctl --user restart n8n barrace-bot`.
`/status` in the bot shows today's use.

## The topic plan and the weekly trend scout

`topics.plan.csv` (in git) is a full year: about 370 topics each for race,
pillars and line charts, 53 per weekly theme, plus 365 quiz rows (`Guess the
Flag: Europe #3` …; the quiz slot switches on with the quiz renderer and
enough quota). Setup copies it to `topics.csv`; after that `topics.csv` is the
live queue (edit it freely).

**Trend scout:** every Monday 08:00 the n8n workflow "barrace weekly trend
scout" runs `daily.py scout`: a sandboxed Claude run searches the news for
timely topics (upcoming tournaments, releases, new data) and proposes up to
5. Each arrives in Telegram with **✅ Add / ❌ Skip**; ✅ puts it at the top of
`topics.csv`, so it is made on its theme's next day. Proposals are checked
like any topics row and never repeat a planned or done title. Files:
`videos/scout/<date>.json` (+ `.log`).

## One-off videos from Telegram

The `barrace-bot` service (`automation/bot.py`) lets you order an extra video
from the Telegram bot, independent of `topics.csv`:

1. Send `/new` to the bot. Tap a style, type a title, optionally send a YouTube
   link (topic hint only) and notes for the research, then tap **Make now**.
2. It runs `daily.py request …`: same research → render → private upload, in
   `videos/<date>-req-<slug>/`. It never reads or changes `topics.csv` or
   `topics_done.csv`; requests are logged in `requests_done.csv`.
3. You get the same ✅/❌ message with Studio links. `/retry` resumes the last
   failed request (finished steps are skipped). `/status` says what is running.

Only one video is made at a time. A request waits for a running video; the
02:00 daily video waits (up to 4 h) for a running request instead of
skipping the day. Keep YouTube's quota in mind: about 3 videos a day in total.
The bot only obeys `TELEGRAM_CHAT_ID`; messages from anyone else are ignored.
Bot logs: `journalctl --user -u barrace-bot`; each request's output:
`videos/.requests/`.

## Running steps by hand

```bash
cd ~/barrace
A="uv run --project automation automation/daily.py"
$A pick                      # prints {"dir": "videos/…"}; takes the lock
$A research videos/<dir>     # Claude writes data, video.toml, metadata.json …
$A render   videos/<dir>     # out/main.mp4, out/short.mp4, out/thumb.png
$A upload   videos/<dir>     # private upload, resumes if interrupted
$A finish   videos/<dir> --status uploaded   # or failed; releases the lock
```

A step that fails writes `videos/<dir>/error.txt`. Claude's full output is in
`videos/<dir>/research.log`; n8n keeps each run's output under Executions.

## Common problems

| message | fix |
|---|---|
| `YouTube login expired or missing` | `ssh -L 8080:localhost:8080 <machine>`, then `$A auth`, open the URL in your browser |
| `claude exited 1` + "Invalid API key"/"login" | `claude setup-token` again; update `CLAUDE_CODE_OAUTH_TOKEN` in `~/.config/barrace/env`; `systemctl --user restart n8n` |
| `usage limit` | the run already waited 30 min and retried once; it'll work on the next run, or run it by hand later |
| `quotaExceeded` | YouTube API allows ~3 runs a day (10,000 units; one run ≈ 3,250). Wait until midnight Pacific |
| `barrace validate failed` | look at `research.log`; re-run with a clearer comment |
| no Telegram message | check `TELEGRAM_BOT_TOKEN` / `TELEGRAM_CHAT_ID` in `~/.config/barrace/env` |
| Telegram "❌ barrace: n8n run … failed" | n8n itself saw the run fail (from the "barrace error alert" workflow). If a pipeline report came just before, it is the same failure; otherwise the script could not start: open n8n → Executions for the output |
| no message at all by ~03:30 | the machine was off/asleep or n8n was down: `systemctl --user status n8n` |
| bot doesn't answer | `systemctl --user status barrace-bot`; `journalctl --user -u barrace-bot -n 30` |
| a run seems stuck / "locked" | a run holds `videos/.lock` (stale after 6 h). Delete it only if no run is active |

## Limits to know

- **YouTube audit:** until Google approves the API project's audit, every
  uploaded video is locked private and *can't* be made public, even by hand.
  Apply at setup time (link in the setup skill).
- **Thumbnails** need a phone-verified channel.
- Research uses your Claude subscription (Sonnet, usually 5–10 min, hard limit 20 min) and
  shares its usage limits with your own Claude use.
