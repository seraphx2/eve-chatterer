import DOMPurify from "dompurify";
import { marked } from "marked";

/**
 * Release notes are the GitHub release's Markdown. They arrive over the
 * network and this window can call every app command, so the HTML is
 * sanitized: no scripts, event handlers, forms or inline styles. Images are
 * dropped too, so showing the notes never fetches anything.
 */
export function renderReleaseNotes(markdown: string): string {
  const html = marked.parse(markdown, { gfm: true, breaks: true, async: false });
  return DOMPurify.sanitize(html, {
    FORBID_TAGS: ["img", "picture", "video", "audio", "iframe", "style", "form", "input", "button"],
    FORBID_ATTR: ["style"],
  });
}
