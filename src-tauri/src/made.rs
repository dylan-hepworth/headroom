// What Claude made in a turn, to look at from the menu bar without opening its chat (see Made.tsx): the documents and
// pictures it wrote, the files it sent the user, and the documents, pictures, and links its reply names.
//
// Only files that are there, and not in a hidden folder like ~/.ssh or a project's .git, are shown. The panel can
// only open, preview, or read what's on a list here (see `Sessions::made_has`).

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// The most a card or a row shows.
const MOST: usize = 8;
/// The most of a document the panel reads.
pub const MOST_READ: u64 = 512 * 1024;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Doc,
    Image,
    Link,
    File,
}

/// What something is: a web address, a document or a picture to look at, or anything else, by its extension.
pub fn kind(item: &str) -> Kind {
    if item.starts_with("http://") || item.starts_with("https://") {
        return Kind::Link;
    }
    let ext = Path::new(item).extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "heif" | "tif" | "tiff" | "bmp" | "svg" => Kind::Image,
        "md" | "markdown" | "txt" | "pdf" | "csv" | "tsv" | "html" | "htm" | "rtf" | "doc" | "docx" | "xls" | "xlsx"
        | "ppt" | "pptx" | "key" | "pages" | "numbers" => Kind::Doc,
        _ => Kind::File,
    }
}

/// A document the panel can show itself, rather than hand to Quick Look.
pub fn readable(path: &str) -> bool {
    let ext = Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    matches!(ext.as_str(), "md" | "markdown" | "txt")
}

/// In a folder that's hidden, like ~/.ssh, ~/.claude, or a project's .git: not for showing, whatever named it.
fn hidden(path: &Path) -> bool {
    path.components().any(|c| {
        let part = c.as_os_str().to_string_lossy();
        part.starts_with('.') && part != "." && part != ".."
    })
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Where a path from a tool or a reply is, if it's a file that's there and not somewhere hidden: from the home folder
/// for "~/…", or from `cwd`, the session's folder, for one that isn't absolute.
pub fn place(path: &str, cwd: &Path) -> Option<String> {
    let path = match path.strip_prefix("~/") {
        Some(rest) => home()?.join(rest),
        None => cwd.join(path),
    };
    let real = std::fs::canonicalize(path).ok()?;
    (real.is_file() && !hidden(&real)).then(|| real.to_string_lossy().into_owned())
}

/// The documents, pictures, and web addresses a reply names, the files among them only if they're there. Code and
/// anything else it mentions stays in the chat.
pub fn named(text: &str, cwd: &Path) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for word in text.split(|c: char| c.is_whitespace() || "`()[]<>\"'|".contains(c)) {
        if out.len() >= MOST {
            break;
        }
        let word = word.trim_start_matches(['*', '_']).trim_end_matches([',', '.', ';', ':', '!', '?', '*', '_']);
        let found = match kind(word) {
            Kind::Link => Some(word.to_string()),
            Kind::Doc | Kind::Image => place(word, cwd),
            Kind::File => None,
        };
        if let Some(found) = found.filter(|f| !out.contains(f)) {
            out.push(found);
        }
    }
    out
}

/// What a session's made this turn, in the order it came, without repeats.
#[derive(Default)]
pub struct Made(Vec<String>);

impl Made {
    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// A file Claude wrote or edited: kept if it's a document or a picture, since code's better read in the chat.
    pub fn wrote(&mut self, path: &str) {
        if matches!(kind(path), Kind::Doc | Kind::Image) {
            self.add(path);
        }
    }

    /// Something already checked over: a file Claude sent (see `place`), or one its reply named (see `named`).
    pub fn add(&mut self, item: &str) {
        if !self.0.iter().any(|i| i == item) {
            self.0.push(item.to_string());
        }
        if self.0.len() > 3 * MOST {
            self.0.remove(0);
        }
    }

    pub fn has(&self, item: &str) -> bool {
        self.0.iter().any(|i| i == item)
    }

    /// For the panel: each one, with `extra` after, that's still there, as what it is and what to call it.
    pub fn to_json(&self, extra: &[String]) -> Value {
        let mut seen: Vec<&str> = vec![];
        let items = self.0.iter().chain(extra).filter(|i| {
            let new = !seen.contains(&i.as_str());
            seen.push(i);
            new
        });
        Value::Array(items.filter_map(|i| item_json(i)).take(MOST).collect())
    }
}

/// One thing Claude made, if it's still there: its path or address, what it is, and a short name for it.
fn item_json(item: &str) -> Option<Value> {
    let kind = kind(item);
    let name = if kind == Kind::Link {
        let rest = item.split_once("://")?.1.trim_end_matches('/');
        let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
        match path.rsplit('/').next().filter(|last| !last.is_empty()) {
            Some(last) if path.contains('/') => format!("{host}/…/{last}"),
            Some(last) => format!("{host}/{last}"),
            None => host.to_string(),
        }
    } else {
        let path = Path::new(item);
        if !path.is_file() || hidden(path) {
            return None;
        }
        path.file_name()?.to_string_lossy().into_owned()
    };
    let kind = match kind {
        Kind::Doc => "doc",
        Kind::Image => "image",
        Kind::Link => "link",
        Kind::File => "file",
    };
    Some(json!({ "path": item, "kind": kind, "name": name }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_names_documents_pictures_and_links_but_not_code_or_hidden_files() {
        let dir = std::env::temp_dir().join(format!("headroom-made-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::create_dir_all(dir.join(".secret")).unwrap();
        for f in ["docs/plan.md", "shot.png", "app.ts", ".secret/keys.txt"] {
            std::fs::write(dir.join(f), "x").unwrap();
        }
        let text = "I wrote up **docs/plan.md**, took `shot.png`, changed app.ts, and read .secret/keys.txt. \
                    See [the docs](https://docs.expo.dev/versions/v57.0.0/sdk/flash-list/). Nothing in gone.md.";
        let found = named(text, &dir);
        let names: Vec<&str> = found.iter().map(|f| f.rsplit('/').next().unwrap()).collect();
        assert_eq!(names, ["plan.md", "shot.png", ""]);
        assert_eq!(found[2], "https://docs.expo.dev/versions/v57.0.0/sdk/flash-list/");
        let shown = Made(found).to_json(&[]);
        assert_eq!(shown[2]["name"], "docs.expo.dev/…/flash-list");
        assert_eq!(shown[1]["kind"], "image");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn only_documents_and_pictures_it_wrote_are_kept() {
        let mut made = Made::default();
        made.wrote("/a/src/app.ts");
        made.wrote("/a/notes.md");
        made.wrote("/a/notes.md");
        assert_eq!(made.0, ["/a/notes.md"]);
    }
}
