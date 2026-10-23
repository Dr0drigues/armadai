---
name: previewing
description: "Open a mockup or a diagram (an .html file under mockups/ or docs/) in the browser through a local preview server bound to 127.0.0.1, or stop that server. Use when the user types /preview, asks to see, open or preview a mockup or a diagram, or asks to stop the preview server."
---
# Previewing

The user judges a mockup or a diagram by seeing it. This procedure serves `mockups/` and `docs/` on the user's machine only and gives a link to open. Run it in the main conversation, never in a subagent: it asks the user a question and starts a process that must outlive the step.

## The choices made here

- Always go through the server, even for a standalone mockup that would open with a double click: one kind of link (`http://127.0.0.1:…`) for every file, clickable in the terminal, and relative links between files keep working. The `file://` path is only the fallback.
- Never serve the project root. `python3 -m http.server` does not check the `Host` header, so a malicious web page can read, by DNS rebinding, whatever it serves on 127.0.0.1: a `.env`, `.armadai/`, the code. The server serves a dedicated folder, `.armadai/preview/`, that holds only two symbolic links, `docs` and `mockups`, and a marker file.

## 1. The file

The project root is the folder that holds `.armadai/`; run every command below from it.

- No argument: the most recently modified `.html` file under `mockups/` or `docs/` (for example `find mockups docs -type f -name '*.html' -exec ls -t {} + 2>/dev/null | head -n 1`). None found: say so, suggest `/mockup`, and stop.
- `stop`: go to step 5.
- A path: that file, relative to the project root or absolute. It must exist, end in `.html` and resolve inside `mockups/` or `docs/` of the project (no `..` leaving them, no symbolic link pointing out of them). A file anywhere else is not served: say why, give the link `file://<absolute path of the file>` to open by hand, and stop.

Keep the path relative to the root, with `/` separators (`mockups/login/v2.html`, `docs/technical/architecture/workspace.html`): it is the end of the URL (encode spaces as `%20`).

## 2. `python3` and the preview folder

Run `python3 --version`. If it fails, say that the preview server needs Python 3, give the `file://` link (a standalone mockup works that way; a diagram also needs the network for Mermaid), and stop. On Windows, give the `file://` link too: the preview folder relies on symbolic links.

Set up `.armadai/preview/`, or check it, so that it holds exactly:

```sh
mkdir -p .armadai/preview
ln -sfn ../../docs .armadai/preview/docs
ln -sfn ../../mockups .armadai/preview/mockups
[ -s .armadai/preview/.project ] || python3 -c "import secrets; print(secrets.token_hex(16))" > .armadai/preview/.project
```

Nothing else goes in this folder. `.project` holds a random token, written once and kept: it tells this project's server apart from another one without revealing anything about the machine. If `.armadai/.gitignore` has no `preview/` line, add one: the folder is local to the machine.

## 3. The server: reuse it, or start one

For each port from 8765 to 8784, in order:

1. Is it ours? `curl -fsS --max-time 2 http://127.0.0.1:<port>/.project | cmp -s - .armadai/preview/.project` succeeds: a server already serves this project. Reuse it, do not start a second one, and go to step 4.
2. Is it free? `python3 -c "import socket; s = socket.socket(); s.bind(('127.0.0.1', <port>))"` succeeds: start the server there, then go to step 4.
3. Otherwise (taken by something else), try the next port.

All twenty ports taken: say so and give the `file://` link instead.

Start the server **in the background**, so the conversation goes on while it runs:

```sh
python3 -m http.server <port> --bind 127.0.0.1 --directory <absolute path of the project root>/.armadai/preview
```

- Claude Code: the Bash tool with `run_in_background: true`.
- OpenCode, or a tool with no background option: `nohup python3 -m http.server <port> --bind 127.0.0.1 --directory <root>/.armadai/preview >/dev/null 2>&1 &`.

Never bind another address (`0.0.0.0`, the machine's IP, `--bind` left out): the files would be readable from the network. Never serve another folder than `.armadai/preview/`. Check that it answers (the `curl` test of step 3.1, retried a few times over a second or two) before giving the link.

## 4. The link, then the browser

Give the link on a line of its own, nothing around it:

```
http://127.0.0.1:<port>/<path of the file>
```

In Claude Code, Cmd+click (macOS) or Ctrl+click opens it. Then ask with your question tool whether to open it in the browser now (yes / no). On yes:

- macOS: `open <url>`
- Linux: `xdg-open <url>`
- Windows: say to open the link by hand.

Then say that, as long as it runs, the server lets any program of this machine (and a web page, by DNS rebinding) read `mockups/` and `docs/`, and that `/preview stop` stops it once the user has finished looking; otherwise it runs until the tool or the machine stops it.

## 5. `stop`

Find the server of this project with the test of step 3.1 on ports 8765 to 8784. None answers: say no preview server is running for this project. One answers:

- if you started it in this conversation as a background task, stop that task with your tool;
- otherwise find its process (`lsof -nP -tiTCP:<port> -sTCP:LISTEN`, or `ps` and the `http.server <port>` command line) and `kill` it, after checking it is the `python3 -m http.server` serving this project's `.armadai/preview`.

Check that the port no longer answers, and say so. Never stop a server that does not serve this project.

Speak the user's language.
