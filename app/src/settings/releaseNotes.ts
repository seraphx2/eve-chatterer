import DOMPurify from "dompurify";
import { marked } from "marked";

/** In the release body, what follows this is the install instructions for
 * the GitHub page (scripts/release-notes.sh); the update screen stops here. */
const INSTALL_MARKER = "<!-- install -->";

/**
 * Release notes are the GitHub release's Markdown. They arrive over the
 * network and this window can call every app command, so the HTML is
 * sanitized: no scripts, event handlers, forms or inline styles. Images are
 * dropped too, so showing the notes never fetches anything. Empty when there
 * is nothing above the install instructions.
 */
export function renderReleaseNotes(markdown: string): string {
  const changes = markdown.split(INSTALL_MARKER)[0];
  if (!changes.trim()) return "";
  const html = marked.parse(changes, { gfm: true, breaks: true, async: false });
  return DOMPurify.sanitize(html, {
    FORBID_TAGS: ["img", "picture", "video", "audio", "iframe", "style", "form", "input", "button"],
    FORBID_ATTR: ["style"],
  });
}
