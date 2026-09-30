// Sanity-check a global-hotkey accelerator before we try to register it.
// Ported from dev-prompt (src/lib/hotkeys.ts); keep the two in step.
//
// Windows has no API to enumerate every app's keyboard shortcuts — window-local
// accelerators (Explorer's Ctrl+Shift+N "New Folder", a browser's Ctrl+T, …)
// are invisible. So this is a curated list, not real detection: block the
// combos the OS reserves or that can't be intercepted, and warn about the ones
// people commonly rely on elsewhere.

export type HotkeyLevel = "ok" | "warn" | "block";
export interface HotkeyVerdict {
  level: HotkeyLevel;
  /** Why, for the "warn" / "block" cases. */
  reason?: string;
}

/** Canonical `sortedMods|KEY` → verdict, for combos we call out by name. */
const NAMED: Record<string, { level: "warn" | "block"; reason: string }> = {
  // Reserved / uninterceptable — block outright.
  "alt|TAB": { level: "block", reason: "Alt+Tab switches windows." },
  "alt|F4": { level: "block", reason: "Alt+F4 closes the active window." },
  "alt|SPACE": { level: "block", reason: "Alt+Space opens the window menu." },
  "ctrl|ESCAPE": { level: "block", reason: "Ctrl+Esc opens the Start menu." },
  "alt+ctrl|DELETE": {
    level: "block",
    reason: "Ctrl+Alt+Del is reserved by Windows.",
  },
  "ctrl+shift|ESCAPE": {
    level: "block",
    reason: "Ctrl+Shift+Esc opens Task Manager.",
  },
  "win|L": { level: "block", reason: "Win+L locks the PC." },
  "win|D": { level: "block", reason: "Win+D shows the desktop." },
  "win|E": { level: "block", reason: "Win+E opens File Explorer." },
  "win|R": { level: "block", reason: "Win+R opens the Run dialog." },
  "win|G": { level: "block", reason: "Win+G opens the Xbox Game Bar." },

  // Commonly used elsewhere — warn and let the user decide.
  "ctrl+shift|N": {
    level: "warn",
    reason: "File Explorer uses Ctrl+Shift+N for New Folder; browsers use it for a private window.",
  },
  "ctrl+shift|T": {
    level: "warn",
    reason: "Browsers and editors use Ctrl+Shift+T to reopen a closed tab.",
  },
  "ctrl+shift|P": {
    level: "warn",
    reason: "Many editors use Ctrl+Shift+P for the command palette.",
  },
  "ctrl+shift|W": {
    level: "warn",
    reason: "Browsers use Ctrl+Shift+W to close the window.",
  },
  "shift+win|S": {
    level: "warn",
    reason: "Win+Shift+S is the Windows screenshot tool.",
  },
  "win|TAB": { level: "warn", reason: "Win+Tab opens Task View." },
  "win|V": { level: "warn", reason: "Win+V opens clipboard history." },
  "alt+ctrl|T": {
    level: "warn",
    reason: "Ctrl+Alt+T opens a terminal on many systems.",
  },
};

const MOD_ALIASES: Record<string, string> = {
  cmdorctrl: "ctrl",
  commandorcontrol: "ctrl",
  ctrl: "ctrl",
  control: "ctrl",
  cmd: "win",
  command: "win",
  super: "win",
  meta: "win",
  win: "win",
  windows: "win",
  alt: "alt",
  option: "alt",
  altgr: "alt",
  shift: "shift",
};

interface Parsed {
  mods: string[];
  key: string;
}

function parse(accel: string): Parsed {
  const parts = accel
    .split("+")
    .map((p) => p.trim())
    .filter(Boolean);
  const key = (parts.pop() ?? "").toUpperCase();
  const mods = [
    ...new Set(parts.map((p) => MOD_ALIASES[p.toLowerCase()] ?? p.toLowerCase())),
  ].sort();
  return { mods, key };
}

const sig = ({ mods, key }: Parsed) => `${mods.join("+")}|${key}`;

export function classifyHotkey(accel: string): HotkeyVerdict {
  const trimmed = accel.trim();
  if (!trimmed) return { level: "block", reason: "No combination set." };

  const p = parse(trimmed);
  const named = NAMED[sig(p)];
  if (named) return named;

  // No modifier: only function keys are workable, and even those clash a lot.
  if (p.mods.length === 0) {
    if (/^F\d{1,2}$/.test(p.key)) {
      return {
        level: "warn",
        reason: "A bare function key often collides with an app shortcut.",
      };
    }
    return {
      level: "block",
      reason: "A global hotkey needs at least one of Ctrl, Alt, Shift or Win.",
    };
  }

  const only = (m: string) => p.mods.length === 1 && p.mods[0] === m;

  if (only("ctrl") && /^[A-Z0-9]$/.test(p.key)) {
    return {
      level: "warn",
      reason: `Ctrl+${p.key} is a standard shortcut in most apps (new, open, save, …).`,
    };
  }
  if (only("alt") && /^[A-Z]$/.test(p.key)) {
    return {
      level: "warn",
      reason: "Alt+letter combinations trigger menu shortcuts in many apps.",
    };
  }
  if (p.mods.includes("win") && p.mods.length === 1) {
    return {
      level: "warn",
      reason: "Win+key combinations are mostly reserved by Windows.",
    };
  }

  return { level: "ok" };
}
