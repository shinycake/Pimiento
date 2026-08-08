#!/usr/bin/env python3
"""Minimal macOS GUI driver for Pimiento (cliclick + System Events)."""

from __future__ import annotations

import argparse
import subprocess
import time
from pathlib import Path

CLICLICK = "/opt/homebrew/bin/cliclick"
PIDFILE = Path("/tmp/pimiento-app.pid")


def sh(args: list[str], check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(args, check=check, text=True, capture_output=True)


def osa(script: str) -> str:
    return sh(["osascript", "-e", script]).stdout.strip()


def ensure_alive() -> int:
    if PIDFILE.exists():
        pid = int(PIDFILE.read_text().strip())
        if sh(["kill", "-0", str(pid)], check=False).returncode == 0:
            return pid
    pid_s = sh(["pgrep", "-n", "pimiento-app"], check=False).stdout.strip()
    if not pid_s:
        raise SystemExit("pimiento-app is not running; start with scripts/run_app.sh")
    return int(pid_s)


def frontmost() -> None:
    deadline = time.monotonic() + 3.0
    while time.monotonic() < deadline:
        if osa('tell application "System Events" to exists process "pimiento-app"') == "true":
            break
        time.sleep(0.1)
    else:
        raise SystemExit("pimiento-app did not appear in System Events within 3 seconds")
    osa('tell application "System Events" to set frontmost of process "pimiento-app" to true')
    time.sleep(0.12)


def window_frame() -> tuple[int, int, int, int]:
    out = osa(
        """
tell application "System Events"
  set p to process "pimiento-app"
  set {wx, wy} to position of window 1 of p
  set {ww, wh} to size of window 1 of p
  return (wx as text) & "," & (wy as text) & "," & (ww as text) & "," & (wh as text)
end tell
"""
    )
    wx, wy, ww, wh = map(int, out.split(","))
    return wx, wy, ww, wh


def position_right_half() -> None:
    frontmost()
    osa(
        """
tell application "Finder"
  set deskBounds to bounds of window of desktop
end tell
set screenW to item 3 of deskBounds
set screenH to item 4 of deskBounds
set halfX to (screenW / 2) as integer
tell application "System Events"
  set p to process "pimiento-app"
  set frontmost of p to true
  set position of window 1 of p to {halfX, 0}
  set size of window 1 of p to {screenW - halfX, screenH}
end tell
"""
    )
    time.sleep(0.25)


def ax_buttons() -> list[tuple[str, int, int, int, int]]:
    raw = osa(
        """
tell application "System Events"
  set p to process "pimiento-app"
  set out to ""
  repeat with e in entire contents of window 1 of p
    try
      if (role of e as text) is "AXButton" then
        set n to ""
        try
          set n to name of e as text
        end try
        set {ex, ey} to position of e
        set {ew, eh} to size of e
        set out to out & n & "|" & ex & "," & ey & "," & ew & "," & eh & linefeed
      end if
    end try
  end repeat
  return out
end tell
"""
    )
    buttons: list[tuple[str, int, int, int, int]] = []
    for line in raw.splitlines():
        if "|" not in line:
            continue
        name, geom = line.split("|", 1)
        x, y, w, h = map(int, geom.split(","))
        buttons.append((name, x, y, w, h))
    return buttons


def click_xy(x: int, y: int) -> None:
    frontmost()
    sh([CLICLICK, f"c:{x},{y}"])


def click_button(name: str) -> None:
    buttons = ax_buttons()
    for n, x, y, w, h in buttons:
        if n == name or name in n:
            click_xy(x + w // 2, y + h // 2)
            return
    # Fallback launcher geometry when AX tree is empty (GPUI often reports 0 elems).
    # Empirically Start here sits near the horizontal mid of the right-half window,
    # slightly above vertical mid of the content card.
    if name == "Start here":
        wx, wy, ww, wh = window_frame()
        click_xy(wx + int(ww * 0.49), wy + int(wh * 0.49))
        return
    raise SystemExit(f"button not found: {name!r}; have={[b[0] for b in buttons]}")


def in_launcher() -> bool:
    names = [n for n, *_ in ax_buttons()]
    if any(n == "Start here" for n in names):
        return True
    # AX empty: treat as launcher only if window title still looks idle/launcher.
    # Title is often blank for GPUI; prefer attempting start when unknown.
    return not names


def send_prompt(text: str) -> None:
    wx, wy, ww, wh = window_frame()
    click_xy(wx + int(ww * 0.45), wy + wh - 40)
    time.sleep(0.2)
    subprocess.run(["pbcopy"], input=text.encode(), check=True)
    sh([CLICLICK, "kd:cmd", "t:a", "ku:cmd"])
    time.sleep(0.05)
    sh([CLICLICK, "kd:cmd", "t:v", "ku:cmd"])
    time.sleep(0.15)
    sh([CLICLICK, "kp:return"])


def toggle_theme() -> None:
    frontmost()
    sh([CLICLICK, "kd:cmd", "t:k", "ku:cmd"])
    time.sleep(0.2)
    # GPUI palette focus can lag briefly after Cmd+K; this remains best-effort.
    sh([CLICLICK, "t:theme"])
    time.sleep(0.15)
    sh([CLICLICK, "kp:return"])


def screenshot(path: str) -> None:
    sh(["screencapture", "-x", path])


def main() -> None:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("ping-alive")
    sub.add_parser("position")
    sub.add_parser("buttons")
    p_click = sub.add_parser("click")
    p_click.add_argument("name")
    sub.add_parser("start-here")
    p_send = sub.add_parser("send")
    p_send.add_argument("text")
    p_shot = sub.add_parser("shot")
    p_shot.add_argument("path")
    sub.add_parser("theme")
    sub.add_parser("smoke")
    args = parser.parse_args()

    ensure_alive()
    if args.cmd == "ping-alive":
        print(ensure_alive())
    elif args.cmd == "position":
        position_right_half()
        print(window_frame())
    elif args.cmd == "buttons":
        for n, x, y, w, h in ax_buttons():
            print(f"{n!r} @{x},{y} {w}x{h}")
    elif args.cmd == "click":
        click_button(args.name)
    elif args.cmd == "start-here":
        position_right_half()
        time.sleep(0.4)
        # Retry AX a few times; GPUI often exposes buttons only after focus/layout.
        for _ in range(6):
            names = [n for n, *_ in ax_buttons()]
            if any(n == "Start here" for n in names):
                click_button("Start here")
                break
            time.sleep(0.25)
        else:
            click_button("Start here")
        time.sleep(3)
        print(window_frame())
    elif args.cmd == "send":
        send_prompt(args.text)
    elif args.cmd == "shot":
        screenshot(args.path)
        print(args.path)
    elif args.cmd == "theme":
        toggle_theme()
        print("theme toggle sent")
    elif args.cmd == "smoke":
        position_right_half()
        time.sleep(0.3)
        if in_launcher() or any(n == "Start here" for n, *_ in ax_buttons()):
            click_button("Start here")
            time.sleep(4)
        send_prompt("ping: reply with exactly PONG and nothing else")
        time.sleep(8)
        screenshot("/tmp/pimiento-smoke.png")
        print("smoke complete → /tmp/pimiento-smoke.png")


if __name__ == "__main__":
    main()
