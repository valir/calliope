export const INVALID_URL_MESSAGE = 'Entered URL is invalid';
const MAX_URL_CHARS = 2000;

/** Mirrors `download::validate_url` in Rust (which validates again): null = fine, else the message. */
export function checkUrl(text: string): string | null {
  const s = text.trim();
  if (s === '' || [...s].length > MAX_URL_CHARS || /[\s\p{Cc}]/u.test(s)) return INVALID_URL_MESSAGE;
  let u: URL;
  try {
    u = new URL(s);
  } catch {
    return INVALID_URL_MESSAGE;
  }
  if (u.protocol !== 'http:' && u.protocol !== 'https:') return INVALID_URL_MESSAGE;
  if (u.hostname === '' || u.username !== '' || u.password !== '') return INVALID_URL_MESSAGE;
  return null;
}
