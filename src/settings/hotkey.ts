/** Turns a browser KeyboardEvent into the "Ctrl+Alt+W" string the Rust shortcut parser accepts. */

const NAMED: Record<string, string> = {
  Space: "Space",
  Tab: "Tab",
  Enter: "Enter",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown",
  Insert: "Insert",
  Delete: "Delete",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  ArrowUp: "Up",
  ArrowDown: "Down",
  Minus: "-",
  Equal: "=",
  BracketLeft: "[",
  BracketRight: "]",
  Semicolon: ";",
  Quote: "'",
  Comma: ",",
  Period: ".",
  Slash: "/",
  Backslash: "\\",
  Backquote: "`",
};

export function keyFromCode(code: string): string | null {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code;
  return NAMED[code] ?? null;
}

export type Recorded =
  | { kind: "pending" } // only modifiers held so far
  | { kind: "invalid"; reason: string }
  | { kind: "ok"; combo: string };

export function recordHotkey(e: KeyboardEvent): Recorded {
  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Super");

  if (["ControlLeft", "ControlRight", "AltLeft", "AltRight", "ShiftLeft", "ShiftRight", "MetaLeft", "MetaRight"].includes(e.code)) {
    return { kind: "pending" };
  }
  const key = keyFromCode(e.code);
  if (!key) return { kind: "invalid", reason: "That key can't be used in a shortcut." };
  const isFKey = /^F\d+$/.test(key);
  if (mods.length === 0 && !isFKey) {
    return { kind: "invalid", reason: "Hold Ctrl, Alt, Shift or Win together with a key." };
  }
  return { kind: "ok", combo: [...mods, key].join("+") };
}
