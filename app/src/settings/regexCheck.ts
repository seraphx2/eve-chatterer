import { invoke } from "@tauri-apps/api/core";

// Tracked patterns are checked by the same regex engine and flags that
// matching uses (Rust's `rules::compile_pattern`); JavaScript's RegExp accepts
// and rejects different things. Answers are cached per pattern.
const cache = new Map<string, Promise<string | null>>();

/** Why `pattern` doesn't compile, or null when it does. */
export function checkRegex(pattern: string): Promise<string | null> {
  let answer = cache.get(pattern);
  if (!answer) {
    answer = invoke<string | null>("check_regex", { pattern }).catch(() => null);
    cache.set(pattern, answer);
  }
  return answer;
}
