// Matching for the settings search. It's forgiving the way a search field in a Mac app should be: a word can be
// started ("notif" for notifications), found inside another ("hold" in threshold), typed with a slip ("notifcation",
// "thershold"), or typed with letters skipped ("thrshld"). Text and query are folded the same way first, with case
// and accents off, and the fold remembers where each character came from, so a match can be shown in bold where it is
// in the original text.

/** Where the query matched, as [start, end) in the original text. */
export type Hit = [number, number];

type Folded = {
  text: string;
  /** For each folded character, the index of the original character it came from. */
  from: number[];
};

function fold(text: string): Folded {
  let out = "";
  const from: number[] = [];
  let i = 0;
  for (const c of text) {
    const f = c.normalize("NFKD").replace(/\p{M}/gu, "").toLowerCase();
    for (let k = 0; k < f.length; k++) from.push(i);
    out += f;
    i += c.length;
  }
  return { text: out, from };
}

const isWordChar = (c: string | undefined) => !!c && /[\p{L}\p{N}]/u.test(c);

/** How many edits (with two letters swapped counting as one) turn `a` into `b`, giving up past `max`. */
function distance(a: string, b: string, max: number): number {
  if (Math.abs(a.length - b.length) > max) return max + 1;
  const d: number[][] = Array.from({ length: a.length + 1 }, (_, i) => [i, ...Array(b.length).fill(0)]);
  for (let j = 0; j <= b.length; j++) d[0][j] = j;
  for (let i = 1; i <= a.length; i++) {
    let rowMin = Infinity;
    for (let j = 1; j <= b.length; j++) {
      const cost = a[i - 1] === b[j - 1] ? 0 : 1;
      d[i][j] = Math.min(d[i - 1][j] + 1, d[i][j - 1] + 1, d[i - 1][j - 1] + cost);
      if (i > 1 && j > 1 && a[i - 1] === b[j - 2] && a[i - 2] === b[j - 1]) d[i][j] = Math.min(d[i][j], d[i - 2][j - 2] + 1);
      rowMin = Math.min(rowMin, d[i][j]);
    }
    if (rowMin > max) return max + 1;
  }
  return d[a.length][b.length];
}

type TokenMatch = { score: number; ranges: Hit[] };

/** How well one word of the query matches some folded text, 0 for not at all. */
function matchToken(token: string, text: string): TokenMatch {
  // The word itself: best as a whole word, then at the start of one, then inside one
  let best: TokenMatch = { score: 0, ranges: [] };
  for (let at = text.indexOf(token); at !== -1; at = text.indexOf(token, at + 1)) {
    const wordStart = at === 0 || !isWordChar(text[at - 1]);
    const wholeWord = wordStart && !isWordChar(text[at + token.length]);
    const score = wholeWord ? 100 : wordStart ? 90 : token.length >= 3 ? 60 : 0;
    if (score > best.score) best = { score, ranges: [[at, at + token.length]] };
  }
  if (best.score > 0) return best;

  // A slip in a word, or in the start of one: one from four letters, two from eight
  const allowed = token.length >= 8 ? 2 : token.length >= 4 ? 1 : 0;
  if (allowed > 0) {
    for (const w of text.matchAll(/[\p{L}\p{N}]+/gu)) {
      const word = w[0];
      const start = w.index ?? 0;
      for (const len of new Set([token.length - 1, token.length, token.length + 1])) {
        if (len < 3 || len > word.length) continue;
        const d = distance(token, word.slice(0, len), allowed);
        if (d <= allowed && 70 - d * 12 > best.score) best = { score: 70 - d * 12, ranges: [[start, start + len]] };
      }
    }
    if (best.score > 0) return best;
  }

  // The word's letters in order, inside one word that starts the same, with few gaps ("thrshld")
  if (token.length >= 3) {
    for (const w of text.matchAll(/[\p{L}\p{N}]+/gu)) {
      const word = w[0];
      const start = w.index ?? 0;
      if (word[0] !== token[0]) continue;
      const ranges: Hit[] = [];
      let at = 0;
      let gaps = 0;
      for (const ch of token) {
        const found = word.indexOf(ch, at);
        if (found === -1) {
          gaps = Infinity;
          break;
        }
        gaps += found - at;
        ranges.push([start + found, start + found + 1]);
        at = found + 1;
      }
      if (gaps <= token.length && 45 - gaps * 3 > best.score) best = { score: 45 - gaps * 3, ranges };
    }
  }
  return best;
}

export type Field = {
  text: string;
  /** How much a match here counts: a title more than a description. */
  weight: number;
  /** Is this the text shown in the results, so its matches are marked? */
  shown?: boolean;
};

/** How well a setting's fields match the query, and where in the shown one. Every word of the query has to match one
 *  of the fields; the score is the average of each word's best. */
export function score(query: string, fields: Field[]): { score: number; hits: Hit[] } {
  const tokens = fold(query.trim()).text.split(/\s+/).filter(Boolean);
  if (tokens.length === 0) return { score: 0, hits: [] };
  const folded = fields.map((f) => ({ ...f, folded: fold(f.text) }));
  let total = 0;
  const hits: Hit[] = [];
  for (const token of tokens) {
    let top = 0;
    let topHits: Hit[] = [];
    for (const f of folded) {
      const m = matchToken(token, f.folded.text);
      if (m.score * f.weight > top) {
        top = m.score * f.weight;
        topHits = f.shown ? m.ranges.map(([a, b]) => [f.folded.from[a], f.folded.from[b - 1] + 1] as Hit) : [];
      }
    }
    if (top === 0) return { score: 0, hits: [] };
    total += top;
    hits.push(...topHits);
  }
  return { score: total / tokens.length, hits };
}

/** Text split into plain and matched pieces, for showing the matches in bold. */
export function pieces(text: string, hits: Hit[]): { text: string; hit: boolean }[] {
  const marks = new Array(text.length).fill(false);
  for (const [a, b] of hits) for (let i = a; i < b && i < text.length; i++) marks[i] = true;
  const out: { text: string; hit: boolean }[] = [];
  for (let i = 0; i < text.length; i++) {
    const last = out[out.length - 1];
    if (last && last.hit === marks[i]) last.text += text[i];
    else out.push({ text: text[i], hit: marks[i] });
  }
  return out;
}
