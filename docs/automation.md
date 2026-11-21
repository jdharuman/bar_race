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

`topics.csv` (repo root, not in git; start from `topics.example.csv`):

```csv
style,title,youtube_url,comment
race,Largest Economies by GDP,,
pillars,Highest Paid NBA Player by Season,https://www.youtube.com/watch?v=q9EjG56OjN4,
race,,https://www.youtube.com/watch?v=gyHSFZ4fIh4,
```

| column | meaning |
|---|---|
| `style` | `race` (bar chart race) or `pillars` (3D pillar walk) |
| `title` | the topic in your words |
| `youtube_url` | optional reference video; used only to learn the topic, never copied |
| `comment` | empty for a new topic; for a redo, what was wrong |

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
   the row back at the top. The next day's run moves on to the next row on its
   own.

Want it now instead of 02:00? In n8n, open "barrace daily video" → "Execute
workflow", or on the machine:
`cd ~/barrace && uv run --project automation automation/daily.py run`.

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
| a run seems stuck / "locked" | a run holds `videos/.lock` (stale after 6 h). Delete it only if no run is active |

## Limits to know

- **YouTube audit:** until Google approves the API project's audit, every
  uploaded video is locked private and *can't* be made public, even by hand.
  Apply at setup time (link in the setup skill).
- **Thumbnails** need a phone-verified channel.
- Research uses your Claude subscription (Opus, up to 45 min per run) and
  shares its usage limits with your own Claude use.
