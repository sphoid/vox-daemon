---
name: Bug Report
about: Report a bug in Vox Daemon
title: ""
labels: bug
assignees: ""
---

**Describe the bug**
A clear and concise description of the bug.

**To reproduce**
Steps to reproduce the behavior:
1. Run `vox-daemon ...`
2. ...

**Expected behavior**
What you expected to happen.

**Environment**
- OS: (e.g., Arch Linux, Ubuntu 24.04)
- Desktop: (e.g., GNOME 46, KDE Plasma 6, Sway 1.9)
- Display server: Wayland / X11
- PipeWire version: (run `pipewire --version`)
- Rust version: (run `rustc --version`)
- Vox Daemon version: (run `vox-daemon --version`)
- GPU: (e.g., NVIDIA RTX 4070, AMD RX 7800 XT)

**Logs**
Run with `-vv` for trace-level logs:
```
vox-daemon -vv record 2>&1 | head -100
```

**Additional context**
Any other context about the problem.
