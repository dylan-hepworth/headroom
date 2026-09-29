// Just enough Markdown for Claude's replies in the popover, and the documents it writes: paragraphs, headings, lists,
// quotes, code, tables, and the usual inline marks. It's turned into elements, never HTML, so nothing in a reply can
// run in the page. Anything else it doesn't know is shown as it was written.

import type { ReactNode } from "react";

type Block =
  | { kind: "p" | "h" | "quote"; text: string }
  | { kind: "code"; text: string }
  | { kind: "list"; ordered: boolean; start: number; items: Block[][] }
  | { kind: "table"; head: string[]; align: ("left" | "center" | "right")[]; rows: string[][] }
  | { kind: "rule" };

/** A table row's cells, without the pipes at either end. */
const cells = (row: string) =>
  row
    .trim()
    .replace(/^\|/, "")
    .replace(/\|$/, "")
    .split("|")
    .map((c) => c.trim());
const TABLE_RULE = /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/;

const LIST_ITEM = /^(\s*)([-*+]|\d+[.)])\s+(.*)$/;
const indent = (line: string) => line.length - line.trimStart().length;

function blocks(text: string): Block[] {
  const lines = text.replace(/\r\n?/g, "\n").split("\n");
  const out: Block[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (!line.trim()) {
      i++;
    } else if (line.trim().startsWith("```")) {
      const code: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trim().startsWith("```")) code.push(lines[i++]);
      i++;
      out.push({ kind: "code", text: code.join("\n") });
    } else if (/^#{1,6}\s/.test(line)) {
      out.push({ kind: "h", text: line.replace(/^#{1,6}\s+/, "") });
      i++;
    } else if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(line)) {
      out.push({ kind: "rule" });
      i++;
    } else if (/^\s*>/.test(line)) {
      const quote: string[] = [];
      while (i < lines.length && /^\s*>/.test(lines[i])) quote.push(lines[i++].replace(/^\s*>\s?/, ""));
      out.push({ kind: "quote", text: quote.join(" ") });
    } else if (/^\s*\|/.test(line)) {
      const rows: string[] = [];
      while (i < lines.length && /^\s*\|/.test(lines[i])) rows.push(lines[i++].trim());
      if (rows.length > 1 && TABLE_RULE.test(rows[1])) {
        const align = cells(rows[1]).map((c) => (c.startsWith(":") && c.endsWith(":") ? "center" : c.endsWith(":") ? "right" : "left"));
        out.push({ kind: "table", head: cells(rows[0]), align, rows: rows.slice(2).map(cells) });
      } else {
        // Not a table after all: as written, lined up in a fixed-width font
        out.push({ kind: "code", text: rows.join("\n") });
      }
    } else if (LIST_ITEM.test(line)) {
      // Each item is its first line, plus whatever's indented under it (more lines, a list inside it, a code block),
      // which is read as Markdown of its own. A blank line doesn't end the list if more of it follows.
      const first = LIST_ITEM.exec(line)!;
      const base = first[1].length;
      const ordered = /\d/.test(first[2]);
      const items: string[][] = [];
      while (i < lines.length) {
        const item = LIST_ITEM.exec(lines[i]);
        if (item && item[1].length === base && /\d/.test(item[2]) === ordered) {
          items.push([item[3]]);
          i++;
        } else if (lines[i].trim() && indent(lines[i]) > base && items.length) {
          items[items.length - 1].push(lines[i].slice(Math.min(indent(lines[i]), base + 3)));
          i++;
        } else if (!lines[i].trim()) {
          let next = i + 1;
          while (next < lines.length && !lines[next].trim()) next++;
          const more = next < lines.length && (indent(lines[next]) > base || LIST_ITEM.exec(lines[next])?.[1].length === base);
          if (!more) break;
          items[items.length - 1]?.push("");
          i = next;
        } else break;
      }
      out.push({ kind: "list", ordered, start: ordered ? parseInt(first[2], 10) : 1, items: items.map((item) => blocks(item.join("\n"))) });
    } else {
      const para: string[] = [];
      while (i < lines.length && lines[i].trim() && !/^(#{1,6}\s|```|\s*>|\s*\|)/.test(lines[i]) && !LIST_ITEM.test(lines[i])) {
        para.push(lines[i++].trim());
      }
      out.push({ kind: "p", text: para.join(" ") });
    }
  }
  return out;
}

/** Code, bold, italics, and links, inside a line. A link shows its words, with the address on hover: clicked, it would
 *  take the popover itself somewhere. */
function inline(text: string): ReactNode[] {
  // Only asterisks for emphasis: underscores are too often part of a name (user_id, __init__)
  const parts = text.split(/(`[^`]+`|\*\*[^*]+\*\*|\*[^*\s][^*]*\*|\[[^\]]+\]\([^)\s]+\))/g);
  return parts.map((part, i) => {
    if (/^`[^`]+`$/.test(part)) return <code key={i}>{part.slice(1, -1)}</code>;
    if (/^\*\*.+\*\*$/.test(part)) return <b key={i}>{inline(part.slice(2, -2))}</b>;
    if (/^\*.+\*$/.test(part)) return <i key={i}>{inline(part.slice(1, -1))}</i>;
    const link = /^\[([^\]]+)\]\(([^)\s]+)\)$/.exec(part);
    if (link) {
      return (
        <span key={i} className="md-link" title={link[2]}>
          {link[1]}
        </span>
      );
    }
    return part;
  });
}

/** A short bit of Markdown as one run of text, for a line in a list: the inline marks shown as they're meant, and the
 *  quote, heading, and list marks at the start of each line left off. */
export function MarkdownSnippet({ text }: { text: string }) {
  let flat = text
    .replace(/\r\n?/g, "\n")
    .split("\n")
    .map((line) =>
      line
        .replace(/^\s*(>\s?)+/, "")
        .replace(/^\s*#{1,6}\s+/, "")
        .replace(/^\s*([-*+]|\d+[.)])\s+/, "")
        .trim(),
    )
    .filter((line) => line && !line.startsWith("```"))
    .join(" ")
    // A quote that starts on a label's line ("**Was:** > …")
    .replace(/(:\*\*|:)\s*>\s/g, "$1 ");
  // Cut off partway, a mark left open goes
  for (const mark of ["**", "`"]) {
    const at = flat.lastIndexOf(mark);
    if (flat.endsWith("…") && flat.split(mark).length % 2 === 0) flat = flat.slice(0, at) + flat.slice(at + mark.length);
  }
  return <span className="md">{inline(flat)}</span>;
}

export function Markdown({ text }: { text: string }) {
  return <div className="md">{render(blocks(text))}</div>;
}

function render(list: Block[]): ReactNode[] {
  return list.map((block, i) => {
    switch (block.kind) {
      case "h":
        return (
          <p key={i} className="md-heading">
            {inline(block.text)}
          </p>
        );
      case "quote":
        return <blockquote key={i}>{inline(block.text)}</blockquote>;
      case "code":
        return (
          <pre key={i}>
            <code>{block.text}</code>
          </pre>
        );
      case "rule":
        return <hr key={i} />;
      case "table":
        return (
          <div key={i} className="md-table">
            <table>
              <thead>
                <tr>
                  {block.head.map((cell, j) => (
                    <th key={j} style={{ textAlign: block.align[j] }}>
                      {inline(cell)}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {block.rows.map((row, r) => (
                  <tr key={r}>
                    {block.head.map((_, j) => (
                      <td key={j} style={{ textAlign: block.align[j] }}>
                        {inline(row[j] ?? "")}
                      </td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        );
      case "list": {
        const List = block.ordered ? "ol" : "ul";
        return (
          <List key={i} start={block.ordered && block.start !== 1 ? block.start : undefined}>
            {block.items.map((item, j) => (
              // An item that's one line of text shows as just that, without a paragraph's spacing
              <li key={j}>{item.length === 1 && item[0].kind === "p" ? inline(item[0].text) : render(item)}</li>
            ))}
          </List>
        );
      }
      default:
        return <p key={i}>{inline(block.text)}</p>;
    }
  });
}
