---
name: barrace-setup
description: Set up the barrace daily video pipeline on a fresh Linux machine - dependencies, Rust build, Claude Code and YouTube login, Telegram, n8n as a service with the daily 02:00 workflow, and a dry run. Use when the user asks to set up, install, move or repair the barrace automation.
---

# barrace setup (fresh Linux machine)

Goal: the repo is cloned at `$REPO` (e.g. `~/barrace`), and n8n runs
`automation/daily.py run` every day at 02:00 as the user, surviving reboots.
Read `CLAUDE.md` and `docs/automation.md` first.

Work through the steps in order and check each one before moving on. The
user must do some steps themselves (logins, Google Cloud console, Telegram):
give exact instructions, then wait. Never print secrets back. Secrets go only in
`~/.config/barrace/` (mode 600), never in the repo.

## 1. System packages

Ubuntu/Debian (adapt for others):

```bash
sudo apt update
sudo apt install -y git curl build-essential pkg-config ffmpeg fonts-noto-core fonts-noto-cjk
ffmpeg -hide_banner -encoders | grep -q libx264 && echo x264 ok
```

## 2. Rust + barrace

```bash
command -v cargo || curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source ~/.cargo/env
cd $REPO && cargo build --release && cargo test --release
```

## 3. uv, Python deps, yt-dlp

```bash
command -v uv || curl -LsSf https://astral.sh/uv/install.sh | sh
cd $REPO/automation && uv sync && uv run pytest -q
uv tool install --force --python 3.12 "yt-dlp[default]"
```

## 4. Node 22 + n8n + Claude Code

```bash
curl -fsSL https://deb.nodesource.com/setup_22.x | sudo -E bash -
sudo apt install -y nodejs
sudo npm install -g n8n
command -v claude || curl -fsSL https://claude.ai/install.sh | bash
```

## 5. Claude login for headless runs (user)

The user runs `claude setup-token` (in their terminal, or `! claude setup-token`
here). It prints a long-lived token for their Claude subscription. Put it in
the env file (step 8) as `CLAUDE_CODE_OAUTH_TOKEN`. Check:
`CLAUDE_CODE_OAUTH_TOKEN=… claude -p "reply with ok"` prints `ok`.

## 6. YouTube API (user, in a browser)

Guide the user through:

1. https://console.cloud.google.com/ → new project "barrace".
2. APIs & Services → Library → enable **YouTube Data API v3**.
3. OAuth consent screen (Google Auth Platform): External; app name; their
   email; add scope `.../auth/youtube.upload`; add their Google account as a
   test user; then **Publish app → In production**. Testing mode tokens
   expire after 7 days. Unverified is fine; they click through the warning
   once.
4. Credentials → Create OAuth client ID → **Desktop app** → download JSON
   → put it at `~/.config/barrace/client_secret.json` (chmod 600).
5. **Apply for the API audit** now:
   https://support.google.com/youtube/contact/yt_api_form. Until it passes,
   API uploads stay locked private (publishing is blocked even by hand).
6. Phone-verify the channel (https://www.youtube.com/verify) so custom
   thumbnails work.

Then the login. The machine is headless, so the user connects from their own
computer with a tunnel:

```bash
ssh -L 8080:localhost:8080 <user>@<this-machine>
cd $REPO && uv run --project automation automation/daily.py auth
```

They open the printed URL in their local browser, pick the channel's
account (choose the brand account if the channel is one), and approve.
`~/.config/barrace/youtube_token.json` appears.

## 7. Telegram (user)

1. In Telegram, talk to @BotFather → `/newbot` → copy the bot token.
2. Send any message to the new bot, then:
   `curl -s https://api.telegram.org/bot<TOKEN>/getUpdates` → `chat.id`.

## 8. Env file

`~/.config/barrace/env` (chmod 600). Use absolute paths only (systemd doesn't
expand `~` or `$HOME` in this file):

```bash
BARRACE_REPO=/home/<user>/barrace
CLAUDE_CODE_OAUTH_TOKEN=<from step 5>
TELEGRAM_BOT_TOKEN=<from step 7>
TELEGRAM_CHAT_ID=<from step 7>
PATH=/home/<user>/.local/bin:/home/<user>/.cargo/bin:/usr/local/bin:/usr/bin:/bin
GENERIC_TIMEZONE=<output of: timedatectl show -p Timezone --value>
TZ=<same>
NODES_EXCLUDE=[]
N8N_SECURE_COOKIE=false
```

`NODES_EXCLUDE=[]` re-enables the Execute Command node (n8n 2.x disables it
by default). `N8N_SECURE_COOKIE=false` lets the user open the UI over a plain
`http://localhost` tunnel.

## 9. n8n as a user service

`~/.config/systemd/user/n8n.service`:

```ini
[Unit]
Description=n8n (barrace daily video)

[Service]
EnvironmentFile=/home/<user>/.config/barrace/env
WorkingDirectory=/home/<user>/barrace
ExecStart=/usr/bin/n8n start
Restart=on-failure

[Install]
WantedBy=default.target
```

```bash
systemctl --user daemon-reload
systemctl --user enable --now n8n
sudo loginctl enable-linger $USER     # keep it running when logged out / after reboot
systemctl --user status n8n --no-pager
```

## 10. Import and turn on the workflow

Tested with n8n 2.48. Run with the env file loaded: without
`NODES_EXCLUDE=[]`, n8n reports `Unrecognized node type: n8n-nodes-base.executeCommand`.

```bash
systemctl --user stop n8n
set -a; . ~/.config/barrace/env; set +a
n8n import:workflow --input=$REPO/n8n/daily-video.json
ID=$(n8n list:workflow | grep "barrace daily video" | cut -d"|" -f1)
n8n publish:workflow --id=$ID          # older 1.x: n8n update:workflow --id=$ID --active=true
n8n list:workflow --active=true        # must list "barrace daily video"
systemctl --user start n8n
```

For the UI (run history, "Execute workflow" button), the user opens a tunnel
(`ssh -L 5678:localhost:5678 …`), goes to http://localhost:5678, and creates the
owner account on first visit.

## 11. Dry run

1. `cp $REPO/topics.example.csv $REPO/topics.csv` (if the user has no list yet),
   or ask them to copy theirs in.
2. In the n8n UI, open the workflow and click **Execute workflow**. Or run it from a shell with the env file loaded, so the Claude token and
   Telegram settings apply:
   `cd $REPO && set -a && . ~/.config/barrace/env && set +a && uv run --project automation automation/daily.py run`.
   It takes 20–60 min.
3. Expect a Telegram message with two Studio links. Open them: both videos
   private, title, description, thumbnail set. Check `topics_done.csv`.
4. If it failed: read `videos/<dir>/error.txt` and `research.log`, fix,
   put the row back at the top of `topics.csv`, run again.

Done when the dry run produced a private pair and the workflow is active.
Tell the user the next run time and remind them about the API audit.

## Moving or updating later

- Update the tool: `cd $REPO && git pull && cargo build --release && (cd automation && uv sync)`.
  `topics.csv`, `topics_done.csv` and `videos/` are not in git, so a pull
  never touches them.
- Moving machines: copy `topics.csv`, `topics_done.csv`, `videos/` (optional)
  and `~/.config/barrace/` to the new machine, then run this skill there.
