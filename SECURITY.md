# Security policy

Clipnest keeps a copy of what you copy, so we take security seriously.

## Reporting a vulnerability

Please **don't** open a public issue for security problems. Instead, use
GitHub's private reporting: the repository's **Security** tab, then
**Report a vulnerability**. You can also reach the maintainers through
https://erag.in.

Include what an attacker could do, how to reproduce it, and the Clipnest version
(`clipnest version`). We aim to reply within a week.

## What Clipnest does with your data

- History is stored only on your computer, in `~/.local/share/clipnest/`
  (SQLite and PNG files). Nothing is sent over the network.
- Copies that password managers mark as secret are not saved, unless you turn
  that off in Preferences → Type.
- Auto-paste needs write access to `/dev/uinput`, which
  `enable-autopaste.sh` grants to the logged-in user through a udev rule. Any
  program you run can then create virtual input devices. Remove
  `/etc/udev/rules.d/70-clipnest-uinput.rules` to undo that.
